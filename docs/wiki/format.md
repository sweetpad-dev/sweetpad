# SweetPad: Format Swift code

This extension integrates [**swift-format**](https://github.com/apple/swift-format) by default, or other formatter of
your choice, with VSCode for formatting Swift files. You can also enable "Format on Save" to format Swift files
automatically when saving.

[![Swift-format](../images/format-demo.gif)](../images/format-demo.gif)

### Installation

To use this feature, first install **swift-format** using Homebrew:

```bash
brew install swift-format
```

Next, add the following configuration to your settings.json file:

```jsonc
{
  "[swift]": {
    "editor.defaultFormatter": "sweetpad.sweetpad",
    "editor.formatOnSave": true
  }
}
```

Then, open your Swift file and press `⌘ + S` to format it 💅🏼

> 🙈 In case of errors, open the Command Palette with `⌘ + P` and run `> SweetPad: Show format logs`. This command will
> open an "Output" panel displaying logs from the formatter. If you encounter issues, grab the logs and open an issue on
> the SweetPad GitHub repository.

### Which formatter to use?

By default, SweetPad is configured to use [**swift-format**](https://github.com/apple/swift-format). This tool is
developed by Apple, so in general we recommend using it.

However, you can use any other formatter of your choice. We provide several configuration options to customize the
formatter used by SweetPad. Here is an example of how to use another formatter
[**swiftformat**](https://github.com/nicklockwood/SwiftFormat):

```jsonc
{
  "sweetpad.format.path": "swiftformat",
  // The "--quiet" flag is important here to ignore output that "swiftformat" writes to stderr.
  // Otherwise, the extension thinks that the formatting failed and shows an annoying error message
  "sweetpad.format.args": ["--quiet", "${file}"]
}
```
