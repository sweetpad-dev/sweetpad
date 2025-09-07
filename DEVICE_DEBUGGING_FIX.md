# Device Debugging Fix - Issue #114

## Problem
When debugging iOS apps on physical devices, the debugger would terminate immediately after the app launched, even though the app itself launched successfully. This prevented users from debugging their iOS applications on real devices.

## Root Cause
The issue was caused by the `--console` flag in the `devicectl device process launch` command. When this flag is used:
- devicectl attaches to the application console and **waits for it to exit**
- This causes the process to complete immediately after launching
- The debugger cannot attach to a process that has already exited
- Result: "Process exited with code -1" immediately after launch

## Solution
Modified the device launch logic to conditionally exclude the `--console` flag when launching in debug mode:

### Key Changes
1. **Added debug parameter**: Added optional `debug?: boolean` to `runOniOSDevice` function signature
2. **Conditional console flag**: Changed launch argument logic from:
   ```typescript
   isConsoleOptionSupported ? "--console" : null
   ```
   to:
   ```typescript
   isConsoleOptionSupported && !option.debug ? "--console" : null
   ```
3. **Propagated debug flag**: Updated all callers to pass the debug flag through the call chain

### Behavior
- **Debug mode** (`debug: true`): Launches without `--console`, keeping process alive for debugger attachment
- **Normal mode** (`debug: false`): Uses `--console` as before for console output capture
- **Legacy Xcode**: Maintains backward compatibility for Xcode < 16

## Files Modified
- `src/build/commands.ts`: Core logic changes and function signature updates
- `src/build/provider.ts`: Pass debug flag to device launch calls
- `src/build/commands.spec.ts`: Unit tests validating the fix

## Testing
Added unit tests to verify:
- Console flag is excluded when `debug=true`
- Console flag is included when `debug=false` 
- Backward compatibility with older Xcode versions
- Proper argument filtering

## Impact
- ✅ Fixes device debugging for iOS physical devices
- ✅ Preserves existing functionality for non-debug launches
- ✅ Maintains backward compatibility
- ✅ No breaking changes to public APIs