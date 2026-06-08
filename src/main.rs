use cyrus_playground::engine::{auto_update_cyrus, execute_cyrus_code, Executor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatMemberKind, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::Mutex;

#[derive(Deserialize, Clone)]
struct AppConfig {
    admin_id: i64,
    bot_username: String,
    group_username: String,
}

impl AppConfig {
    fn load() -> Self {
        let content = fs::read_to_string("configs/config.toml")
            .expect("Failed to read configs/config.toml. Please ensure it exists.");
        toml::from_str(&content).expect("Failed to parse configs/config.toml")
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct BotState {
    enabled: bool,
    allowed_topics: HashMap<String, bool>,
    topic_names: HashMap<String, String>,
    group_chat_id: Option<i64>,
}

impl BotState {
    fn load() -> Self {
        if let Ok(content) = fs::read_to_string("configs/state.toml") {
            if let Ok(state) = toml::from_str(&content) {
                return state;
            }
        }
        // Return default state if file doesn't exist or is invalid
        Self {
            enabled: true,
            allowed_topics: HashMap::new(),
            topic_names: HashMap::new(),
            group_chat_id: None,
        }
    }

    fn save(&self) {
        if let Ok(content) = toml::to_string_pretty(self) {
            let _ = fs::create_dir_all("configs");
            if let Err(e) = fs::write("configs/state.toml", content) {
                log::error!("Failed to write to configs/state.toml: {}", e);
            }
        } else {
            log::error!("Failed to serialize BotState");
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    log::info!("Starting Cyrus Playground Bot");

    let bot_token =
        std::env::var("CYRUS_BOT_TOKEN").expect("CYRUS_BOT_TOKEN environment variable not set");
    let bot = Bot::new(bot_token);

    // Initialize config and state
    let app_config = Arc::new(AppConfig::load());
    let state = Arc::new(Mutex::new(BotState::load()));
    let executor = Arc::new(Mutex::new(Executor::new()));

    let executor_clone = Arc::clone(&executor);
    tokio::spawn(async move {
        auto_update_cyrus(executor_clone).await;
    });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![executor, app_config, state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    app_config: Arc<AppConfig>,
    state: Arc<Mutex<BotState>>,
) -> ResponseResult<()> {
    let user_id = query.from.id.0 as i64;

    if user_id != app_config.admin_id {
        bot.answer_callback_query(query.id.clone()).await?;
        return Ok(());
    }

    if let Some(data) = &query.data {
        let mut st = state.lock().await;

        match data.as_str() {
            "enable_bot" => {
                st.enabled = true;
                st.save();
                bot.answer_callback_query(query.id.clone())
                    .text("✅ Bot enabled")
                    .await?;
            }
            "disable_bot" => {
                st.enabled = false;
                st.save();
                bot.answer_callback_query(query.id.clone())
                    .text("❌ Bot disabled")
                    .await?;
            }
            "enable_all_topics" => {
                st.allowed_topics.clear();
                st.save();
                bot.answer_callback_query(query.id.clone())
                    .text("✅ All topics enabled")
                    .await?;
            }
            "list_topics" => {
                bot.answer_callback_query(query.id.clone()).await?;

                if let Some(chat_id) = st.group_chat_id {
                    let _ = fetch_forum_topics(&bot, chat_id, &mut st).await;
                }

                show_topics_menu(&bot, &query, &st).await?;
                return Ok(());
            }
            "back_to_main" => {
                bot.answer_callback_query(query.id.clone()).await?;
                show_main_menu(&bot, &query, &st).await?;
                return Ok(());
            }
            _ if data.starts_with("toggle_topic_") => {
                if let Ok(topic_id) = data.strip_prefix("toggle_topic_").unwrap().parse::<i32>() {
                    let key = topic_id.to_string();
                    let is_enabled = st.allowed_topics.get(&key).copied().unwrap_or(true);
                    st.allowed_topics.insert(key, !is_enabled);
                    st.save();

                    let status_text = if !is_enabled { "enabled" } else { "disabled" };
                    bot.answer_callback_query(query.id.clone())
                        .text(format!("Topic {} {}", topic_id, status_text))
                        .await?;

                    show_topics_menu(&bot, &query, &st).await?;
                    return Ok(());
                }
            }
            _ => {}
        }

        drop(st);
        let st = state.lock().await;
        show_main_menu(&bot, &query, &st).await?;
    }

    Ok(())
}

async fn fetch_forum_topics(
    _bot: &Bot,
    _chat_id: i64,
    _state: &mut BotState,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Fetching forum topics for chat_id: {}", _chat_id);
    Ok(())
}

async fn show_main_menu(bot: &Bot, query: &CallbackQuery, state: &BotState) -> ResponseResult<()> {
    let status = if state.enabled {
        "🟢 Enabled"
    } else {
        "🔴 Disabled"
    };

    let topics_status = if state.allowed_topics.is_empty() {
        "All topics enabled".to_string()
    } else {
        let enabled_count = state.allowed_topics.values().filter(|&&v| v).count();
        format!("{} topics enabled", enabled_count)
    };

    let text = format!(
        "🤖 *Cyrus Playground Bot*\n\n\
        Status: {}\n\
        Topics: {}\n\n\
        Choose an option:",
        status, topics_status
    );

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("🟢 Enable Bot", "enable_bot"),
            InlineKeyboardButton::callback("🔴 Disable Bot", "disable_bot"),
        ],
        vec![
            InlineKeyboardButton::callback("📋 Manage Topics", "list_topics"),
            InlineKeyboardButton::callback("🌐 Enable All Topics", "enable_all_topics"),
        ],
    ]);

    if let Some(message) = &query.message {
        if let Some(msg) = message.regular_message() {
            bot.edit_message_text(msg.chat.id, msg.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
        }
    }

    Ok(())
}

async fn show_topics_menu(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
) -> ResponseResult<()> {
    let mut buttons = vec![];

    if state.topic_names.is_empty() {
        let text = "*📋 Topic Management*\n\n\
                    No topics discovered yet\\.\n\n\
                    To discover topics:\n\
                    1\\. Send a message mentioning the bot in each topic\n\
                    2\\. Topics will appear here automatically\n\n\
                    Example: `@cyrus_playground_bot test`";

        buttons.push(vec![InlineKeyboardButton::callback(
            "⬅️ Back",
            "back_to_main",
        )]);

        let keyboard = InlineKeyboardMarkup::new(buttons);

        if let Some(message) = &query.message {
            if let Some(msg) = message.regular_message() {
                bot.edit_message_text(msg.chat.id, msg.id, text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(keyboard)
                    .await?;
            }
        }
        return Ok(());
    }

    let mut topic_ids: Vec<(i32, String)> = state
        .topic_names
        .iter()
        .filter_map(|(tid_str, name)| tid_str.parse::<i32>().ok().map(|tid| (tid, name.clone())))
        .collect();

    topic_ids.sort_by_key(|(tid, _)| *tid);

    for (topic_id, topic_name) in topic_ids {
        let key = topic_id.to_string();
        let is_enabled = state.allowed_topics.get(&key).copied().unwrap_or(true);
        let icon = if is_enabled { "✅" } else { "❌" };
        let label = format!("{} {}", icon, topic_name);

        buttons.push(vec![InlineKeyboardButton::callback(
            label,
            format!("toggle_topic_{}", topic_id),
        )]);
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "⬅️ Back",
        "back_to_main",
    )]);

    let keyboard = InlineKeyboardMarkup::new(buttons);

    let text = "*📋 Topic Management*\n\nClick to toggle topics on/off:";

    if let Some(message) = &query.message {
        if let Some(msg) = message.regular_message() {
            bot.edit_message_text(msg.chat.id, msg.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
        }
    }

    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    executor: Arc<Mutex<Executor>>,
    app_config: Arc<AppConfig>,
    state: Arc<Mutex<BotState>>,
) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if msg.chat.is_private() {
        if user_id == app_config.admin_id {
            handle_admin_private(&bot, &msg, state).await?;
        }
        return Ok(());
    }

    let chat_username = msg.chat.username();
    log::info!("Message in group: {:?}", chat_username);

    if let Some(username) = chat_username {
        if username != app_config.group_username {
            log::info!("Ignoring message from group: {}", username);
            return Ok(());
        }
    } else {
        log::info!("Group has no username, ignoring");
        return Ok(());
    }

    if let Some(text) = msg.text() {
        let trimmed = text.trim();

        if trimmed == "/settopic" {
            if let Some(user) = &msg.from {
                let member = bot.get_chat_member(msg.chat.id, user.id).await?;
                let is_admin = matches!(
                    member.kind,
                    ChatMemberKind::Owner(_) | ChatMemberKind::Administrator(_)
                );

                if !is_admin {
                    bot.send_message(msg.chat.id, "Only group admins can use this command.")
                        .reply_parameters(teloxide::types::ReplyParameters::new(msg.id))
                        .await?;
                    return Ok(());
                }

                if let Some(topic_id) = msg.thread_id {
                    let mut st = state.lock().await;
                    let key = topic_id.0 .0.to_string();

                    st.allowed_topics.insert(key.clone(), true);

                    let topic_name = format!("Topic {}", topic_id.0 .0);
                    st.topic_names.insert(key, topic_name.clone());
                    st.save();

                    bot.send_message(msg.chat.id, format!("✅ Bot enabled in {}", topic_name))
                        .reply_parameters(teloxide::types::ReplyParameters::new(msg.id))
                        .await?;

                    log::info!("Topic {} enabled by admin", topic_id.0 .0);
                } else {
                    bot.send_message(msg.chat.id, "This command only works in topics.")
                        .reply_parameters(teloxide::types::ReplyParameters::new(msg.id))
                        .await?;
                }
                return Ok(());
            }
        }
    }

    let mut st = state.lock().await;

    if st.group_chat_id.is_none() {
        st.group_chat_id = Some(msg.chat.id.0);
        st.save();
        log::info!("Set group chat ID: {}", msg.chat.id.0);
    }

    if !st.enabled {
        log::info!("Bot is disabled");
        return Ok(());
    }

    if let Some(topic_id) = msg.thread_id {
        log::info!("Message in topic: {:?}", topic_id);
        let key = topic_id.0 .0.to_string();
        let is_enabled = st.allowed_topics.get(&key).copied().unwrap_or(false);
        if !is_enabled {
            log::info!("Topic {:?} is not enabled", topic_id);
            return Ok(());
        }
    } else {
        log::info!("Message not in a topic, ignoring");
        return Ok(());
    }
    drop(st);

    if let Some(text) = msg.text() {
        let trimmed = text.trim();
        log::info!("Received text: {}", trimmed);

        if trimmed.is_empty() {
            return Ok(());
        }

        let should_execute = if let Some(entities) = msg.entities() {
            entities.iter().any(|e| {
                if e.kind == teloxide::types::MessageEntityKind::Mention {
                    let start = e.offset;
                    let end = start + e.length;
                    let mention = &trimmed[start..end];
                    log::info!("Found mention: {}", mention);
                    mention == app_config.bot_username
                } else {
                    false
                }
            })
        } else {
            false
        };

        log::info!("Should execute: {}", should_execute);

        if should_execute {
            let code = trimmed
                .replace(&app_config.bot_username, "")
                .trim()
                .to_string();
            log::info!("Code to execute: {}", code);
            if !code.is_empty() {
                execute_and_reply(&bot, &msg, &code, executor).await?;
            }
        }
    }

    Ok(())
}

async fn handle_admin_private(
    bot: &Bot,
    msg: &Message,
    state: Arc<Mutex<BotState>>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        let trimmed = text.trim();

        if trimmed == "/start" || trimmed == "/menu" {
            let st = state.lock().await;
            let status = if st.enabled {
                "🟢 Enabled"
            } else {
                "🔴 Disabled"
            };

            let topics_status = if st.allowed_topics.is_empty() {
                "All topics enabled".to_string()
            } else {
                let enabled_count = st.allowed_topics.values().filter(|&&v| v).count();
                format!("{} topics enabled", enabled_count)
            };

            let text = format!(
                "🤖 *Cyrus Playground Bot*\n\n\
                Status: {}\n\
                Topics: {}\n\n\
                Choose an option:",
                status, topics_status
            );

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🟢 Enable Bot", "enable_bot"),
                    InlineKeyboardButton::callback("🔴 Disable Bot", "disable_bot"),
                ],
                vec![
                    InlineKeyboardButton::callback("📋 Manage Topics", "list_topics"),
                    InlineKeyboardButton::callback("🌐 Enable All Topics", "enable_all_topics"),
                ],
            ]);

            bot.send_message(msg.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
        } else if trimmed == "/debug" {
            let st = state.lock().await;
            let topics: Vec<String> = st
                .allowed_topics
                .iter()
                .map(|(tid, enabled)| {
                    format!("Topic {}: {}", tid, if *enabled { "✅" } else { "❌" })
                })
                .collect();

            let debug_info = format!(
                "Debug Info:\n\
                Enabled: {}\n\
                Tracked topics: {}\n\n{}",
                st.enabled,
                st.allowed_topics.len(),
                if topics.is_empty() {
                    "No topics tracked".to_string()
                } else {
                    topics.join("\n")
                }
            );

            bot.send_message(msg.chat.id, debug_info).await?;
        }
    }

    Ok(())
}

async fn execute_and_reply(
    bot: &Bot,
    msg: &Message,
    code: &str,
    executor: Arc<Mutex<Executor>>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let message_id = msg.id;

    let sent_msg = bot
        .send_message(chat_id, "`Running...`")
        .parse_mode(ParseMode::MarkdownV2)
        .reply_parameters(teloxide::types::ReplyParameters::new(message_id))
        .await?;

    let sent_msg_id = sent_msg.id;
    let bot_clone = bot.clone();
    let code_owned = code.to_string();

    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let bot_for_updates = bot_clone.clone();
        let update_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;
                let elapsed = start_time.elapsed().as_secs_f64();
                let dots = ".".repeat((elapsed as usize % 3) + 1);
                let status = format!("`Running{} ({:.1}s)`", dots, elapsed);
                let _ = bot_for_updates
                    .edit_message_text(chat_id, sent_msg_id, status)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
            }
        });

        match execute_cyrus_code(executor, &code_owned).await {
            Ok(result) => {
                update_handle.abort();

                let status = if result.success {
                    format!("Success ({:.2}s)", result.execution_time)
                } else {
                    format!("Failed ({:.2}s)", result.execution_time)
                };

                let mut output = String::new();

                let stdout_clean = result
                    .stdout
                    .lines()
                    .filter(|line| !line.contains("compiled ") && !line.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");

                let stderr_clean = result
                    .stderr
                    .lines()
                    .filter(|line| !line.contains("compiled ") && !line.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");

                if !stdout_clean.is_empty() {
                    output.push_str(&stdout_clean);
                }
                if !stderr_clean.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&stderr_clean);
                }

                let formatted = if output.len() > 3500 {
                    format!(
                        "<blockquote expandable>{}</blockquote>\n\n<b>{}</b>",
                        escape_html(&output[..3500]),
                        escape_html(&status)
                    )
                } else if output.len() > 200 {
                    format!(
                        "<blockquote expandable>{}</blockquote>\n\n<b>{}</b>",
                        escape_html(&output),
                        escape_html(&status)
                    )
                } else if !output.is_empty() {
                    format!(
                        "<pre>{}</pre>\n\n<b>{}</b>",
                        escape_html(&output),
                        escape_html(&status)
                    )
                } else {
                    format!("<b>{}</b>", escape_html(&status))
                };

                let _ = bot_clone
                    .edit_message_text(chat_id, sent_msg_id, formatted)
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Err(e) => {
                update_handle.abort();
                let _ = bot_clone
                    .edit_message_text(chat_id, sent_msg_id, format!("<b>{}</b>", escape_html(&e)))
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
    });

    Ok(())
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
