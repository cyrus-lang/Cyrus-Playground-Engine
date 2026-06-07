use cyrus_playground::*;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode, ThreadId, ChatMemberKind};
use tokio::sync::Mutex;

const ADMIN_ID: i64 = 5889545873;
const BOT_USERNAME: &str = "@cyrus_playground_bot";

struct BotConfig {
    enabled: bool,
    allowed_topics: HashMap<ThreadId, bool>,
    topic_names: HashMap<ThreadId, String>,
    group_chat_id: Option<i64>,
}

impl BotConfig {
    fn new() -> Self {
        Self {
            enabled: true,
            allowed_topics: HashMap::new(),
            topic_names: HashMap::new(),
            group_chat_id: None,
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    log::info!("Starting Cyrus Playground Bot");

    let bot_token = std::env::var("TELOXIDE_TOKEN")
        .expect("TELOXIDE_TOKEN environment variable not set");
    let bot = Bot::new(bot_token);
    
    let executor = Arc::new(Mutex::new(CyrusExecutor::new()));
    let config = Arc::new(Mutex::new(BotConfig::new()));

    let executor_clone = Arc::clone(&executor);
    tokio::spawn(async move {
        auto_update_cyrus(executor_clone).await;
    });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![executor, config])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    config: Arc<Mutex<BotConfig>>,
) -> ResponseResult<()> {
    let user_id = query.from.id.0 as i64;

    if user_id != ADMIN_ID {
        bot.answer_callback_query(query.id.clone()).await?;
        return Ok(());
    }

    if let Some(data) = &query.data {
        let mut cfg = config.lock().await;

        match data.as_str() {
            "enable_bot" => {
                cfg.enabled = true;
                bot.answer_callback_query(query.id.clone())
                    .text("✅ Bot enabled")
                    .await?;
            }
            "disable_bot" => {
                cfg.enabled = false;
                bot.answer_callback_query(query.id.clone())
                    .text("❌ Bot disabled")
                    .await?;
            }
            "enable_all_topics" => {
                cfg.allowed_topics.clear();
                bot.answer_callback_query(query.id.clone())
                    .text("✅ All topics enabled")
                    .await?;
            }
            "list_topics" => {
                bot.answer_callback_query(query.id.clone()).await?;
                
                if let Some(chat_id) = cfg.group_chat_id {
                    let _ = fetch_forum_topics(&bot, chat_id, &mut cfg).await;
                }
                
                show_topics_menu(&bot, &query, &cfg).await?;
                return Ok(());
            }
            "back_to_main" => {
                bot.answer_callback_query(query.id.clone()).await?;
                show_main_menu(&bot, &query, &cfg).await?;
                return Ok(());
            }
            _ if data.starts_with("toggle_topic_") => {
                if let Ok(topic_id) = data.strip_prefix("toggle_topic_").unwrap().parse::<i32>() {
                    let topic_thread_id = ThreadId(MessageId(topic_id));
                    let is_enabled = cfg.allowed_topics.get(&topic_thread_id).copied().unwrap_or(true);
                    cfg.allowed_topics.insert(topic_thread_id, !is_enabled);

                    let status = if !is_enabled { "enabled" } else { "disabled" };
                    bot.answer_callback_query(query.id.clone())
                        .text(format!("Topic {} {}", topic_id, status))
                        .await?;

                    show_topics_menu(&bot, &query, &cfg).await?;
                    return Ok(());
                }
            }
            _ => {}
        }

        drop(cfg);
        let cfg = config.lock().await;
        show_main_menu(&bot, &query, &cfg).await?;
    }

    Ok(())
}

async fn fetch_forum_topics(_bot: &Bot, _chat_id: i64, _config: &mut BotConfig) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Fetching forum topics for chat_id: {}", _chat_id);
    Ok(())
}

async fn show_main_menu(
    bot: &Bot,
    query: &CallbackQuery,
    config: &BotConfig,
) -> ResponseResult<()> {
    let status = if config.enabled {
        "🟢 Enabled"
    } else {
        "🔴 Disabled"
    };

    let topics_status = if config.allowed_topics.is_empty() {
        "All topics enabled".to_string()
    } else {
        let enabled_count = config.allowed_topics.values().filter(|&&v| v).count();
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
    config: &BotConfig,
) -> ResponseResult<()> {
    let mut buttons = vec![];

    if config.topic_names.is_empty() {
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

    let mut topic_ids: Vec<(ThreadId, String)> = config.topic_names
        .iter()
        .map(|(tid, name)| (*tid, name.clone()))
        .collect();
    
    topic_ids.sort_by_key(|(tid, _)| tid.0.0);

    for (topic_id, topic_name) in topic_ids {
        let is_enabled = config.allowed_topics.get(&topic_id).copied().unwrap_or(true);
        let icon = if is_enabled { "✅" } else { "❌" };
        let label = format!("{} {}", icon, topic_name);

        buttons.push(vec![InlineKeyboardButton::callback(
            label,
            format!("toggle_topic_{}", topic_id.0.0),
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
    executor: Arc<Mutex<CyrusExecutor>>,
    config: Arc<Mutex<BotConfig>>,
) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if msg.chat.is_private() {
        if user_id == ADMIN_ID {
            handle_admin_private(&bot, &msg, config).await?;
        }
        return Ok(());
    }

    let chat_username = msg.chat.username();
    log::info!("Message in group: {:?}", chat_username);
    
    if let Some(username) = chat_username {
        if username != "cyrus_lang" {
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
                    let mut config_lock = config.lock().await;
                    config_lock.allowed_topics.insert(topic_id, true);
                    
                    let topic_name = format!("Topic {}", topic_id.0.0);
                    config_lock.topic_names.insert(topic_id, topic_name.clone());
                    
                    bot.send_message(
                        msg.chat.id,
                        format!("✅ Bot enabled in {}", topic_name)
                    )
                    .reply_parameters(teloxide::types::ReplyParameters::new(msg.id))
                    .await?;
                    
                    log::info!("Topic {} enabled by admin", topic_id.0.0);
                } else {
                    bot.send_message(msg.chat.id, "This command only works in topics.")
                        .reply_parameters(teloxide::types::ReplyParameters::new(msg.id))
                        .await?;
                }
                return Ok(());
            }
        }
    }

    let mut config_lock = config.lock().await;
    
    if config_lock.group_chat_id.is_none() {
        config_lock.group_chat_id = Some(msg.chat.id.0);
        log::info!("Set group chat ID: {}", msg.chat.id.0);
    }
    
    if !config_lock.enabled {
        log::info!("Bot is disabled");
        return Ok(());
    }

    if let Some(topic_id) = msg.thread_id {
        log::info!("Message in topic: {:?}", topic_id);
        let is_enabled = config_lock
            .allowed_topics
            .get(&topic_id)
            .copied()
            .unwrap_or(false);
        if !is_enabled {
            log::info!("Topic {:?} is not enabled", topic_id);
            return Ok(());
        }
    } else {
        log::info!("Message not in a topic, ignoring");
        return Ok(());
    }
    drop(config_lock);

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
                    mention == BOT_USERNAME
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
                .replace(BOT_USERNAME, "")
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
    config: Arc<Mutex<BotConfig>>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        let trimmed = text.trim();
        
        if trimmed == "/start" || trimmed == "/menu" {
            let cfg = config.lock().await;
            let status = if cfg.enabled {
                "🟢 Enabled"
            } else {
                "🔴 Disabled"
            };

            let topics_status = if cfg.allowed_topics.is_empty() {
                "All topics enabled".to_string()
            } else {
                let enabled_count = cfg.allowed_topics.values().filter(|&&v| v).count();
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
            let cfg = config.lock().await;
            let topics: Vec<String> = cfg.allowed_topics.iter()
                .map(|(tid, enabled)| format!("Topic {}: {}", tid.0.0, if *enabled { "✅" } else { "❌" }))
                .collect();
            
            let debug_info = format!(
                "Debug Info:\n\
                Enabled: {}\n\
                Tracked topics: {}\n\n{}",
                cfg.enabled,
                cfg.allowed_topics.len(),
                if topics.is_empty() { "No topics tracked".to_string() } else { topics.join("\n") }
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
    executor: Arc<Mutex<CyrusExecutor>>,
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
                
                let stdout_clean = result.stdout.lines()
                    .filter(|line| !line.contains("compiled ") && !line.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                
                let stderr_clean = result.stderr.lines()
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
