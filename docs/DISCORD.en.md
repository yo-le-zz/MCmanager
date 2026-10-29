# Discord bot

> Version française : [DISCORD.md](DISCORD.md)

Notifications, `/mc` commands and, **for each Minecraft server, a dedicated Discord category** with:

| Channel | Purpose |
|---|---|
| `status` | notifications for THIS server (starting, ready, stopped, crash, backup…) |
| `console` | **live** console output; authorized roles **type commands** there |
| `chat` | in-game chat **⇄** Discord (both directions) |

Settings: **Settings → 🤖 Discord bot**. The bot connects *out* to Discord: no port to open, no domain needed. It runs inside the
MCManager process (`mcmanager` or `mcmanager-server`) and stays idle until enabled.

## Setup

1. <https://discord.com/developers/applications> → **New Application** → **Bot** → copy the **token**. Still in *Bot* →
   *Privileged Gateway Intents*: enable **Message Content Intent** (needed for the console and chat inside Discord; without it,
   notifications and `/mc` commands still work).
2. In MCManager paste the token and click **🔍 Detect**: the **invite link** appears (with every permission needed: create
   channels and their permissions, write, react). Invite the bot, click *Detect* again: your Discord server, its **roles**
   (colored checkboxes) and **channels** (menu) are loaded — no IDs to copy.
3. **Who can do what?** Tick the *operator* roles (start/stop, console commands), the roles that can *read* the console, and the
   ones with access to the *chat*. **By default nobody can control anything**: pick at least one operator role.
4. **Channels per Minecraft server**: **✨ Create channels** for each server. Choose live console, chat bridge and, if needed,
   extra roles allowed to type commands *on that server only*.
5. Tick **Enable**, save, **📨 Send a test**.

## Console in Discord

- Server lines arrive in batches every 2 s in `console`, as code blocks (colors stripped). At most 3 messages per batch: when logs
  flood, an “N line(s) skipped” summary avoids saturating Discord.
- A message typed in `console` by an authorized role is sent as a **command** (leading `/` optional). Reactions: ✅ sent, ❌ not
  possible (server stopped), ⛔ role not allowed, ⚠️ multi-line message (one command per message).
- Who can type: **operators** (roles or users) + the roles added for that server. “Viewer” roles see the console but cannot
  write — Discord forbids it **and** MCManager re-checks on every message.

## Minecraft ⇄ Discord chat

- Game → Discord: `**Alex** » hello` in `chat`. Joins/leaves stay in `status`.
- Discord → game: shown to all players as `[Discord] <Nickname> message` (`tellraw`). Text is sanitized (line breaks, `§` codes),
  240 characters max, and cannot inject another command.
- No bot message can ever ping `@everyone`, `@here` or a role.

## `/mc` commands

Declared in English, **shown in French** to French Discord clients; replies follow the user's language.

| Command | Who | Effect |
|---|---|---|
| `/mc list` | everyone | servers and state 🟢/⚫ (remote instances included) |
| `/mc status server` | everyone | online? players / max |
| `/mc players server` | everyone | connected players |
| `/mc start server` | **operators** | start |
| `/mc stop server` | **operators** | clean stop |
| `/mc restart server` | **operators** | restart |
| `/mc command server text` | **operators** | console command |

## Language

*Bot language* (French / English): notifications and messages posted in channels. Replies to `/mc` follow each person's Discord
client language (French → French, anything else → English).

## Security

- **Deny by default**: with no operator configured, nobody starts, stops or sends commands.
- Channels are created **hidden from `@everyone`**, visible only to the chosen roles. Write access is **re-checked** by MCManager on
  every message (never rely on channel permissions alone).
- Token and webhook are **encrypted on disk** and never sent back to the UI. Commands only work in *your* Discord server.
- Every control action is logged with the person's Discord ID.
- An operator can send **any** command (`op`, `stop`…): only grant it to people you trust.
- The bot **never** asks for the Administrator permission.

## Troubleshooting

- **“Console and chat unavailable… Message Content Intent”**: enable the privileged intent in the developer portal (step 1), then save again.
- **Channel creation: 403 / “Missing Permissions”**: invite the bot with the provided link (it includes *Manage Channels* and *Manage Roles*),
  and make sure **the bot's role is above** the roles it must manage (Server Settings → Roles).
- **“bot token rejected (4004)”**: token pasted with a space, or regenerated since.
- **`/mc` does not show up**: bot invited without `applications.commands` → re-invite it with the provided link.
- A server's events are notified by the instance **it runs on**; for a remote server, also configure the bot (or a webhook) on that instance.
  Console and chat of a remote server are relayed by the instance where the bot is configured.
