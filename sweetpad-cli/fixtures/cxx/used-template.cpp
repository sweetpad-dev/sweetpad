// Reduced from the MMKV Debug generated-header failure.
// Apple Clang accepts this; Linux Swift 6.4.2 Clang recurses even for iOS.
template<class T> struct Optional {
 __attribute__((used)) void bridge() { Optional<Optional<T>> result; result.pointer(); }
 __attribute__((used)) void pointer() {}
};
Optional<int> value;
