//
// FixtureChoice.swift
//
// This file was automatically generated and should not be edited.
//

#if canImport(Intents)

import Intents

@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)
@objc public enum FixtureChoice: Int {
    case `unknown` = 0
    case `public` = 1
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *) @available(tvOS, unavailable)
@objc(FixtureChoiceResolutionResult)
public class FixtureChoiceResolutionResult: INEnumResolutionResult {

    // This resolution result is for when the app extension wants to tell Siri to proceed, with a given FixtureChoice. The resolvedValue can be different than the original FixtureChoice. This allows app extensions to apply business logic constraints.
    // Use notRequired() to continue with a 'nil' value.
    @objc(successWithResolvedFixtureChoice:)
    public class func success(with resolvedValue: FixtureChoice) -> Self {
        return __success(withResolvedValue: resolvedValue.rawValue)
    }

    // This resolution result is to ask Siri to confirm if this is the value with which the user wants to continue.
    @objc(confirmationRequiredWithFixtureChoiceToConfirm:)
    public class func confirmationRequired(with valueToConfirm: FixtureChoice) -> Self {
        return __confirmationRequiredWithValue(toConfirm: valueToConfirm.rawValue)
    }
}

#endif
