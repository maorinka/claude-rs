Get or set Claude Code configuration settings.

  View or change Claude Code settings. Use when the user requests configuration changes, asks about current settings, or when adjusting a setting would benefit them.


## Usage
- **Get current value:** Omit the "value" parameter
- **Set new value:** Include the "value" parameter

## Configurable settings list
The following settings are available for you to change:

### Global Settings (stored in ~/.claude.json)
- theme: "dark", "dark-daltonized", "dark-ansi", "light", "light-daltonized", "light-ansi" - Set the color theme for Claude Code.
- verbose: true/false - Enable verbose output mode.
- editorMode: "emacs", "vim" - Set the input editor mode.
- preferredNotifChannel: "none", "iterm2", "iterm2_with_bell", "terminal_bell", "notifications_disabled" - Preferred notification channel.
- permissions.defaultMode: "default", "acceptEdits", "plan", "bypassPermissions" - Default permission mode for tool execution.
- autoUpdates: true/false - Enable automatic updates.
- includeCoAuthoredBy: true/false - (DEPRECATED — use `attribution.commit`) Include Claude as co-author in git commits.

### Project Settings (stored in settings.json)
- spinnerTipsEnabled: true/false - Show tips in the spinner while Claude is thinking.
- hasAcknowledgedCostThreshold: true/false - Whether the user has acknowledged the cost threshold warning.

## Model
- model - Override the default model (sonnet, opus, haiku, best, or full model ID)

## Examples
- Get theme: { "setting": "theme" }
- Set dark theme: { "setting": "theme", "value": "dark" }
- Enable vim mode: { "setting": "editorMode", "value": "vim" }
- Enable verbose: { "setting": "verbose", "value": true }
- Change model: { "setting": "model", "value": "opus" }
- Change permission mode: { "setting": "permissions.defaultMode", "value": "plan" }
