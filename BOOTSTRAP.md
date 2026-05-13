# PRISM CLI Bootstrap

## Overview

When you run `prism.exe` from any location (like your Downloads folder, a USB drive, etc.), it automatically:

1. **Detects** if it's running from `C:\Program Files\PRISM\`
2. **Installs** itself to `C:\Program Files\PRISM\` if it's not
3. **Adds** `C:\Program Files\PRISM\` to your system PATH
4. **Relaunches** from the installed location with your original command

This ensures `prism` is always accessible from any command prompt after the first run.

## First Run Experience

```bash
# First run - from Downloads folder
C:\Users\You\Downloads> prism.exe -c my-widget.html

[PRISM] Installing to C:\Program Files\PRISM\...
[PRISM] ✓ Installed to C:\Program Files\PRISM\prism.exe
[PRISM] ✓ Added to system PATH
[PRISM] Relaunching from installed location...

[PRISM] Compiling: "my-widget.html"
[PRISM] ✓ Compiled to: my-widget.prd
```

## Subsequent Runs

```bash
# After first run - from anywhere
# Works because prism is in PATH and in Program Files

C:\Users\You> prism -c cpu.html
[PRISM] Compiling: "cpu.html"
[PRISM] ✓ Compiled to: cpu.prd

# Or from any directory with a widget
cd widgets/
prism -r
[PRISM] Auto-detected: cpu.prd
[PRISM] Opening: cpu.prd
```

## How It Works

### Detection

The bootstrap checks if the current executable path contains `C:\Program Files\PRISM\`:

```rust
if !current_dir.contains(&"c:\\program files\\prism") {
    // Need to bootstrap
}
```

### Installation

1. Creates `C:\Program Files\PRISM\` directory
2. Copies the executable to that location
3. Attempts to add the directory to user PATH (`HKCU\Environment\Path`)

### PATH Addition

- Modifies `HKEY_CURRENT_USER\Environment\Path`
- No elevation/administrator required (user PATH, not system PATH)
- Automatically restarts your terminal on next launch to pick up new PATH

### Relaunch

Spawns a new process with the installed executable and passes all original arguments:

```
prism.exe -c my-widget.html
    ↓ (installation happens)
C:\Program Files\PRISM\prism.exe -c my-widget.html
    ↓ (original command continues)
[PRISM] Compiling: "my-widget.html"
```

## Manual PATH Setup

If PATH addition fails during bootstrap, you can set it up manually:

```bash
prism --setup-env
```

This shows instructions for manually adding to PATH:

1. Press `Win+X`, select "System"
2. Click "Advanced system settings"
3. Click "Environment Variables"
4. Under "User variables", click "New"
5. Set: `Path = C:\Program Files\PRISM`
6. Restart your terminal

## Troubleshooting

**Q: Why does my terminal restart after first run?**
- It doesn't. The bootstrap relaunches the command, then exits. Your terminal continues normally.

**Q: Can I uninstall?**
- Yes, just delete `C:\Program Files\PRISM\`. You can also remove the PATH entry manually in Environment Variables.

**Q: Does it require administrator?**
- No. `Program Files\PRISM\` is created automatically when needed. User PATH doesn't require elevation.

**Q: What if I have permission issues?**
- If the directory can't be created, the bootstrap will fail with an error message. Try running as administrator or using a different installation path.

**Q: Can I run from a custom location?**
- Yes, just keep the executable there and it won't bootstrap. But you'll need to add it to PATH manually or use the full path every time.
