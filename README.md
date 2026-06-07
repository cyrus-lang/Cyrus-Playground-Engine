# Cyrus Playground Bot & API

A Telegram bot and REST API for executing Cyrus programming language code using the latest binary from GitHub Actions artifacts.

## Components

1. **Telegram Bot** - Executes code in @cyrus_lang group
2. **REST API** - HTTP API for web/mobile apps (see [API.md](API.md))
3. **Shared Library** - Common execution logic

## Features

- Download latest Cyrus binary from GitHub Actions artifacts (no GitHub token required!)
- Execute Cyrus code through Telegram
- **Telegram Bot**: Only works in @cyrus_lang group
- **Admin Control**: Enable/disable bot and set topic restrictions via inline keyboard buttons
- **Smart Mentions**: Only responds when bot is mentioned with `@cyrus_playground_bot`
- **Clean Output**: Uses expandable quote blocks for long outputs (no group spam)
- **API**: Available for web/mobile apps (optional, see [API.md](API.md))
- Supports topic/forum groups with per-topic control via button panel
- **Automatic updates**: Checks for new Cyrus builds every 12 hours
- **Smart caching**: Only downloads if a new version is available
- **Fast execution**: Async processing with instant message updates

## Setup

1. **Install Rust** (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Clone and build**:
   ```bash
   cd cyrus-playground-bot
   cargo build --release
   ```

3. **Configure environment variables**:
   ```bash
   cp .env.example .env
   ```
   
   Edit `.env` and add:
   - `TELOXIDE_TOKEN`: Your Telegram bot token from [@BotFather](https://t.me/botfather)
   - `PORT`: API server port (default: 3000)
   - `RUST_LOG`: Log level (info, debug, error)

4. **Bot Configuration**
   - Admin ID: Set in source code (src/main.rs)
   - Group: @cyrus_lang
   - Only admin can control bot via private chat

## Usage

### Run Telegram Bot

```bash
source .env
cargo run --bin cyrus-bot --release
```

Or:
```bash
./target/release/cyrus-bot
```

### Run API Server

```bash
source .env
cargo run --bin cyrus-api --release
```

Or:
```bash
./target/release/cyrus-api
```

### Run Both (separate terminals)

Terminal 1:
```bash
source .env
cargo run --bin cyrus-bot --release
```

Terminal 2:
```bash
source .env
cargo run --bin cyrus-api --release
```

## Admin Control Panel (Private Chat)

Admin (ID: 7474145303) can control the bot via **inline keyboard buttons** in private chat.

### Access Control Panel
Send `/start` or `/menu` to the bot in private chat.

### Features:
- 🟢 **Enable Bot** - Turn bot on
- 🔴 **Disable Bot** - Turn bot off  
- 📋 **Manage Topics** - View and toggle individual topics
- 🌐 **Enable All Topics** - Allow all topics at once

### Topic Management:
- Click on any topic to toggle it on/off
- ✅ = Topic enabled
- ❌ = Topic disabled
- Real-time status updates

## How to Use in Group

**Mention the bot** with your code:

```
@cyrus_playground_bot
import std::libc{printf};

pub fn main() {
    printf("Hello, Cyrus!\n");
}
```

The bot will:
1. Execute your code instantly
2. Reply with a collapsed quote for long outputs
3. Show execution time and status
4. Update the message in real-time

### Output Formatting:
- **Short output (<200 chars)**: Regular code block
- **Medium output (200-3500 chars)**: Expandable quote block
- **Long output (>3500 chars)**: Truncated expandable quote block
- **Status**: Shows ✅ Success or ❌ Failed with execution time

## Telegram Bot Usage

**Mention the bot** with your Cyrus code in @cyrus_lang group:

```
@cyrus_playground_bot
import std::libc{printf};

pub fn main() {
    printf("Hello, Cyrus!\n");
}
```

Bot will execute and show results with:
- ✅ Success or ❌ Failed status
- Execution time
- Expandable quotes for long outputs
- Real-time updates

## API Usage

See [API.md](API.md) for complete API documentation.

Quick example:
```bash
curl -X POST http://localhost:3000/api/execute \
  -H "Content-Type: application/json" \
  -d '{"code": "import std::libc{printf};\n\npub fn main() {\n    printf(\"Hello!\\n\");\n}"}'
```

## Example Output

When you send code, the response is:
```
✅ Success (0.23s)

Hello, Cyrus!
```

All output is displayed in clean code blocks.

## Group Setup (Telegram)

The bot only responds in the **@cyrus_lang** group (or supergroup).

To use:
1. Add the bot to @cyrus_lang group
2. Make sure the bot has permission to read messages
3. Group admins use `/settopic` in each topic to enable the bot
4. Works with topics (forum-style groups)

## How It Works

1. **On Startup**: Automatically downloads the latest Cyrus binary + stdlib
2. **Update Checks**: Every 12 hours, checks GitHub for new successful builds
3. **Smart Updates**: Only downloads if run_id has changed
4. **Execution**: Uses nightly.link service to access public artifacts without authentication
5. **Command**: Runs `cyrus run <file>` (stdlib is auto-detected from artifact)

## GitHub Artifacts Limits

- **nightly.link (no token)**: ~60 requests/hour per IP
- **Update frequency**: Every 12 hours (well within limits)
- **Download size**: Typically 5-50 MB per artifact
- **No issues**: Smart caching means downloads only when new version exists

## Requirements

- Rust 1.70+
- Telegram Bot Token (for bot)
- Internet connection

## Security Notes

- The bot executes code in temporary files
- **Telegram Bot**: Only responds in @cyrus_lang group for security
- **API**: Open to all (add authentication/rate limiting for production)
- Consider implementing additional sandboxing for production use
- Limit execution time and resource usage

## Production Deployment

For production API deployment:
- Use reverse proxy (Nginx/Caddy)
- Add rate limiting
- Implement proper sandboxing
- Add monitoring and metrics
- Use HTTPS

See [API.md](API.md) for details.

## License

MIT
