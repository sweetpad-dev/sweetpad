# Troubleshooting

## See logs

1. Set up "debug" level logging in the configuration file to show debug messages in the output panel.

```json
{
  "sweetpad.system.logLevel": "debug"
}
```

2. Restart Visual Studio Code to apply the changes.

3. Check the "Output" panel for debug messages.

![debug](../images/troubleshooting-output-panel.png)

## Reset extension cache

The extension caches user choices such as the simulator to run on, configuration to use, or active workspace. If you
encounter any issues or just want to make another choice, you can reset the cache by running the
`> SweetPad: Reset Extension Cache` command from the command palette.
