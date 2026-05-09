# Telegram Channel Setup

Connect a Telegram bot to your Moatless instance so users can send messages via Telegram and receive agent responses.

## Prerequisites

- A running Moatless backend with a PostgreSQL database
- A Telegram bot token (from [@BotFather](https://t.me/BotFather))

## 1. Create a Telegram Bot

1. Open Telegram and start a chat with [@BotFather](https://t.me/BotFather)
2. Send `/newbot` and follow the prompts to choose a name and username
3. BotFather replies with a **bot token** (e.g. `123456:ABC-DEF1234...`). Save this token.

## 2. Configure the Backend

Set the following environment variables:

```bash
TELEGRAM_BOT_TOKEN=123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11
```

### Ingress Mode

The backend supports two ways to receive messages from Telegram:

| Mode | Env Var | When to Use |
|------|---------|-------------|
| **Webhook** (default) | `TELEGRAM_MODE=webhook` | Server is publicly reachable by Telegram |
| **Polling** | `TELEGRAM_MODE=polling` | Server is behind NAT/firewall, no public URL |

#### Webhook Mode (default)

Telegram pushes updates to your server via HTTP POST. You need:

- A public URL that Telegram can reach (e.g. `https://your-domain.com`)
- The backend's webhook endpoint exposed at `/api/v1/channels/telegram/webhook`

Set up a webhook with the Telegram API:

```bash
curl -X POST "https://api.telegram.org/bot<BOT_TOKEN>/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://your-domain.com/api/v1/channels/telegram/webhook", "secret_token": "your-secret"}'
```

The `secret_token` should match the `secret_token` in your channel config's `provider_config` (see step 3).

#### Polling Mode

The backend calls Telegram's `getUpdates` API in a loop (long-polling). No inbound network access required.

```bash
TELEGRAM_MODE=polling
```

When polling starts, the backend automatically deletes any existing webhook so `getUpdates` works. The poller uses 30-second long-poll requests with exponential backoff on errors (1s to 30s).

> **Note:** Only one instance can poll at a time. If two instances both use polling mode, Telegram returns 409 Conflict for the second one.

## 3. Create a Channel Config

After the backend is running, create a channel config via the API to route Telegram messages to a repository:

### Private Chats (DMs)

To accept messages from any user who DMs the bot, use `*` as the `source_id`:

```bash
curl -X POST http://localhost:8080/api/v1/channels/configs \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <API_KEY>" \
  -d '{
    "provider": "telegram",
    "sourceId": "*",
    "repositoryId": "<REPOSITORY_ID>",
    "routingMode": "by_subject",
    "providerConfig": {
      "bot_username": "@your_bot_username"
    }
  }'
```

### Specific Chat or Group

To listen to a specific chat, use the chat ID as `source_id`:

```bash
curl -X POST http://localhost:8080/api/v1/channels/configs \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <API_KEY>" \
  -d '{
    "provider": "telegram",
    "sourceId": "-1001234567890",
    "repositoryId": "<REPOSITORY_ID>",
    "routingMode": "by_subject",
    "providerConfig": {
      "bot_username": "@your_bot_username",
      "secret_token": "your-secret"
    }
  }'
```

> **Tip:** To find a chat ID, send a message to the bot and check the Telegram API: `curl https://api.telegram.org/bot<TOKEN>/getUpdates` -- look for `chat.id` in the response.

### Provider Config Options

| Field | Type | Description |
|-------|------|-------------|
| `bot_username` | string | Bot's `@username` -- used to strip mentions in group messages |
| `secret_token` | string | Webhook secret for verifying inbound requests (webhook mode only) |

### Routing Modes

| Mode | Behavior |
|------|----------|
| `ongoing` | All messages route to a single long-lived task |
| `by_subject` | Each external subject gets one task; replies on the same subject reuse it. Subject granularity is provider-specific (Telegram: chat in DMs, message in groups) |

### Threading Behavior

- **Private chats (DMs):** All messages from the same user are threaded into one persistent task (using the chat ID as thread key).
- **Group chats:** Each new message starts its own task. Replies (via Telegram's reply feature) continue the existing task's thread.

## 4. Map Telegram Users to System Users

By default, messages from unmapped Telegram users are ignored. To map a Telegram user to a system user, insert a row in the `provider_tokens` table:

```sql
INSERT INTO provider_tokens (provider, provider_user_id, user_id)
VALUES ('telegram', '123456789', 'your-system-user-id');
```

Where `123456789` is the Telegram user's numeric ID (visible in the `from.id` field of Telegram updates).

## 5. Verify

### Webhook mode

Send a message to your bot in Telegram. Check the backend logs for:

```
Telegram webhook received
Telegram message routed for processing
```

### Polling mode

Check the backend logs for:

```
Telegram poller started
Telegram poller: cleared existing webhook
```

Then send a message to the bot. You should see:

```
Telegram message parsed
Telegram message routed for processing
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| No messages received (webhook) | Verify the webhook URL is publicly reachable. Check `curl https://api.telegram.org/bot<TOKEN>/getWebhookInfo` for errors. |
| No messages received (polling) | Check logs for "Telegram poller started". Ensure no other instance is polling (409 Conflict). |
| "Telegram user not mapped" | Add the user's Telegram ID to the `provider_tokens` table. |
| Bot ignores messages in groups | Ensure `bot_username` is set in `provider_config` and the bot is mentioned (or is an admin with access to all messages via BotFather's `/setprivacy` -> Disable). |
| Duplicate messages | The provider deduplicates by `update_id` with a 5-minute cache. If restarting frequently, duplicates may occur within the first batch of pending updates -- they are harmless. |

## Environment Variables Reference

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | Yes | -- | Bot token from BotFather |
| `TELEGRAM_MODE` | No | `webhook` | `webhook` or `polling` |
