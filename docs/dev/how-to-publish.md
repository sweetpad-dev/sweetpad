https://code.visualstudio.com/api/working-with-extensions/publishing-extension#publish-an-extension

https://marketplace.visualstudio.com/manage/publishers/sweetpad/extensions/sweetpad/hub

https://open-vsx.org/extension/sweetpad/sweetpad

# Auto-increment version number

1. Update the version number in `package.json` and publish the extension to VSCode Marketplace.
```shell
vsce publish patch
```

2. Package the extension and upload it to the Open VSX Registry.
```shell
vsce package
```

https://open-vsx.org/user-settings/extensions

3. Push the changes to the GitHub repository.
```shell
git push origin main
```

