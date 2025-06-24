# Roon Waybar Module

A fast, reliable waybar module for Roon music control with real-time updates.

## ✨ Features

- 🎵 **Real-time track display** - Instant updates when songs change
- ⚡ **Lightning-fast controls** - Play/pause/next/previous with < 10ms response
- 🔄 **Persistent connection** - No connection delays, works offline-first  
- 🎯 **Perfect waybar integration** - JSON output with CSS class support
- 🛡️ **Enterprise-grade reliability** - Smart reconnection, timeouts, and graceful recovery

## 🚀 Quick Start

### 1. Build and Install

```bash
# Clone the repository
git clone <your-repo-url>
cd roon-waybar

# Build release version (optimized)
cargo build --release

# Install to your PATH
cp target/release/roon-waybar ~/.local/bin/
# or system-wide:
# sudo cp target/release/roon-waybar /usr/local/bin/
```

### 2. Initial Setup

```bash
# One-time authorization with Roon
roon-waybar setup

# If Roon Core is on the same machine:
roon-waybar setup --ip 127.0.0.1
```

This will prompt you to authorize the extension in your Roon app. Go to **Roon Settings > Extensions** and enable "Waybar module for Roon control".

### 3. Start the Daemon

```bash
# Start daemon in background
nohup roon-waybar --daemon > /dev/null 2>&1 &

# Or for debugging:
roon-waybar --daemon
```

### 4. Test It Works

```bash
# Get current status
roon-waybar status
# Should return: {"class":"playing","text":"♪ Track Name",...}

# Test transport controls
roon-waybar pause
roon-waybar play
```

## 🎵 Waybar Configuration

Add this to your waybar `config.json`:

```json
{
    "custom/roon": {
        "exec": "roon-waybar status",
        "interval": 1,
        "format": "{}",
        "return-type": "json",
        "on-click": "roon-waybar playpause",
        "on-click-right": "roon-waybar next",
        "on-click-middle": "roon-waybar previous"
    }
}
```

### CSS Styling

Add to your waybar `style.css`:

```css
#custom-roon {
    padding: 0 10px;
    border-radius: 3px;
}

#custom-roon.playing {
    color: #a6e3a1;  /* Green when playing */
}

#custom-roon.paused {
    color: #f9e2af;  /* Yellow when paused */
}

#custom-roon.connecting {
    color: #89b4fa;  /* Blue when connecting */
}

#custom-roon.loading {
    color: #fab387;  /* Orange when loading zones */
}

#custom-roon.disconnected {
    color: #f38ba8;  /* Red when disconnected */
}
```

## 🔧 Management Commands

```bash
# Check daemon status
ps aux | grep "roon-waybar --daemon"

# Stop daemon
pkill -f "roon-waybar --daemon"

# View daemon logs (if not using /dev/null)
tail -f daemon.log

# Re-authorize if needed
roon-waybar setup --ip 127.0.0.1
```

## 🚀 Auto-Start with Systemd (Optional)

Create `~/.config/systemd/user/roon-waybar.service`:

```ini
[Unit]
Description=Roon Waybar Daemon
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/roon-waybar --daemon
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

Enable and start:

```bash
systemctl --user daemon-reload
systemctl --user enable roon-waybar.service
systemctl --user start roon-waybar.service

# Check status
systemctl --user status roon-waybar.service
```

## 🛠️ Available Commands

| Command | Description |
|---------|-------------|
| `roon-waybar status` | Get current track info (for waybar) |
| `roon-waybar play` | Start playback |
| `roon-waybar pause` | Pause playback |
| `roon-waybar playpause` | Toggle play/pause |
| `roon-waybar next` | Skip to next track |
| `roon-waybar previous` | Previous track |
| `roon-waybar setup [--ip IP]` | Initial authorization |
| `roon-waybar --daemon` | Run persistent daemon |

## 🔍 Troubleshooting

**"daemon is not running"**
```bash
roon-waybar --daemon &
```

**"Not connected to Roon"**
```bash
# Re-run setup
roon-waybar setup --ip 127.0.0.1
# Then restart daemon
```

**Transport controls not working**
- Ensure daemon is running and connected
- Check Roon Extensions settings for authorization
- Verify zone has playback capability (not just streaming input)

**High CPU usage**
- Make sure you're using the release build: `cargo build --release`
- Check daemon logs for connection issues

**Status shows "Connecting..." or "Loading zones..."**
- This is normal during startup - the daemon uses granular connection states
- Should transition to "Ready" within 30 seconds
- If stuck, check Roon Core accessibility and authorization

## 📋 Requirements

- **Rust** (for building)
- **Roon Core** on your network
- **Waybar** (obviously!)
- **Linux** (tested on Arch, should work on any Unix-like system)

## 🏗️ How It Works

This module uses a **daemon architecture**:

1. **Daemon** (`--daemon`) maintains a persistent connection to Roon API
2. **Client commands** (`status`, `play`, etc.) communicate with daemon via Unix socket
3. **Real-time updates** stream from Roon Core (track changes, seek position)
4. **Instant responses** - no connection delays for waybar

This provides the responsiveness of a native desktop app with the simplicity waybar modules expect.

### 🔧 Enhanced Reliability Features

- **Granular Connection States**: Clear status progression (Connecting → Connected → Ready)
- **Smart Retry Logic**: Exponential backoff prevents server overload (2s → 5min)  
- **Comprehensive Timeouts**: Zone subscription (15s), transport commands (10s)
- **Zone Intelligence**: Prefers zones with active tracks and enabled controls
- **Data Validation**: Ensures zone completeness before Ready state
- **Graceful Degradation**: Clear error messages and fallback behavior

## 📖 Documentation

- [CLAUDE.md](./CLAUDE.md) - Comprehensive technical documentation
- [Waybar Wiki](https://github.com/Alexays/Waybar/wiki) - General waybar configuration
- [Roon API](https://github.com/TheAppgineer/rust-roon-api) - Underlying Roon integration

---

**Enjoy your music! 🎵**