# How to publish VSCode extension

## Links
 - https://code.visualstudio.com/api/working-with-extensions/publishing-extension#publish-an-extension
 - https://marketplace.visualstudio.com/manage/publishers/sweetpad/extensions/sweetpad/hub
 - https://open-vsx.org/extension/sweetpad/sweetpad

##  Steps

1. Update & commit CHANGELOG.md

2. Update the version number and publish to Github: 
```shell
npm run publish-patch
```

3. Check publishing status on github actions page:
 - https://github.com/sweetpad-dev/sweetpad/actions

4. Verify the GitHub release page includes the VSIX asset and the changelog section for the tag:
 - https://github.com/sweetpad-dev/sweetpad/releases
