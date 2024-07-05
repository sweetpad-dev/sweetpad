# SweetPad: Tuist integration

Instead of managing your Xcode project directly in Xcode, you can use the [Tuist](https://tuist.io) tool to define the
structure of your project in a declarative way. The SweetPad extension provides integration with Tuist for the most
common commands.

## Commands

- `sweetpad.tuist.generate` - SweetPad: Generate an Xcode project using Tuist
- `sweetpad.tuist.install` - SweetPad: Install Swift Package using Tuist
- `sweetpad.tuist.clean` - SweetPad: Clean Tuist project
- `sweetpad.tuist.edit` - SweetPad: Edit Tuist project (Open project in Xcode)

## Watcher

The SweetPad extension includes a file watcher for Tuist projects. When you create or delete a ".swift" file in the
project, the watcher will automatically regenerate the Xcode project by executing the `sweetpad.tuist.generate` command.

To enable the watcher, add the following configuration to your `settings.json` file:

```json
{
  "sweetpad.tuist.watcher": true
}
```
