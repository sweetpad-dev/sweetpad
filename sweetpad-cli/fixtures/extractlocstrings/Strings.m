void strings(void) {
 WMFLocalizedString(@"objc-two", @"Two comment");
 WMFLocalizedString(@"objc-three", @"Value", @"Three comment");
 WMFLocalizedStringWithDefaultValue(@"objc-default", @"Table", bundle, @"Default", @"Default comment");
}
