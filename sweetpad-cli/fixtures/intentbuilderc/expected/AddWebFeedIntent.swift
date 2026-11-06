//
// AddWebFeedIntent.swift
//
// This file was automatically generated and should not be edited.
//

#if canImport(Intents)

import Intents

@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)
@objc(AddWebFeedIntent)
public class AddWebFeedIntent: INIntent {

    @NSManaged public var url: URL?
    @NSManaged public var accountName: String?
    @NSManaged public var folderName: String?

}

/*!
 @abstract Protocol to declare support for handling a AddWebFeedIntent. By implementing this protocol, a class can provide logic for resolving, confirming and handling the intent.
 @discussion The minimum requirement for an implementing class is that it should be able to handle the intent. The confirmation method is optional. The handling method is always called last, after confirming the intent.
 */
@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)
@objc(AddWebFeedIntentHandling)
public protocol AddWebFeedIntentHandling: NSObjectProtocol {

    @available(*, renamed: "handle(intent:)")
    @objc(handleAddWebFeed:completion:)
    func handle(intent: AddWebFeedIntent, completion: @escaping (AddWebFeedIntentResponse) -> Swift.Void)
    
    /*!
     @abstract Handling method - Execute the task represented by the AddWebFeedIntent that's passed in
     @discussion Called to actually execute the intent. The app must return a response for this intent.

     @param  intent The input intent
     @param  completion The response handling block takes a AddWebFeedIntentResponse containing the details of the result of having executed the intent

     @see  AddWebFeedIntentResponse
     */
    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(handleAddWebFeed:completion:)
    func handle(intent: AddWebFeedIntent) async -> AddWebFeedIntentResponse

    /*!
     @abstract Resolution methods - Determine if this intent is ready for the next step (confirmation)
     @discussion Called to make sure the app extension is capable of handling this intent in its current form. This method is for validating if the intent needs any further fleshing out.

     @param  intent The input intent
     @param  completion The response block contains an INIntentResolutionResult for the parameter being resolved

     @see INIntentResolutionResult
     */
    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @available(*, renamed: "resolveUrl(for:)")
    @objc(resolveUrlForAddWebFeed:withCompletion:)
    func resolveUrl(for intent: AddWebFeedIntent, with completion: @escaping (AddWebFeedUrlResolutionResult) -> Swift.Void)

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(resolveUrlForAddWebFeed:withCompletion:)
    func resolveUrl(for intent: AddWebFeedIntent) async -> AddWebFeedUrlResolutionResult

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @available(*, renamed: "resolveAccountName(for:)")
    @objc(resolveAccountNameForAddWebFeed:withCompletion:)
    func resolveAccountName(for intent: AddWebFeedIntent, with completion: @escaping (AddWebFeedAccountNameResolutionResult) -> Swift.Void)

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(resolveAccountNameForAddWebFeed:withCompletion:)
    func resolveAccountName(for intent: AddWebFeedIntent) async -> AddWebFeedAccountNameResolutionResult

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @available(*, renamed: "resolveFolderName(for:)")
    @objc(resolveFolderNameForAddWebFeed:withCompletion:)
    func resolveFolderName(for intent: AddWebFeedIntent, with completion: @escaping (AddWebFeedFolderNameResolutionResult) -> Swift.Void)

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(resolveFolderNameForAddWebFeed:withCompletion:)
    func resolveFolderName(for intent: AddWebFeedIntent) async -> AddWebFeedFolderNameResolutionResult

    /*!
     @abstract Dynamic options methods - provide options for the parameter at runtime
     @discussion Called to query dynamic options for the parameter and this intent in its current form.

     @param  intent The input intent
     @param  completion The response block contains options for the parameter
     */
    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @available(*, renamed: "provideAccountNameOptions(for:)")
    @objc(provideAccountNameOptionsForAddWebFeed:withCompletion:)
    optional func provideAccountNameOptions(for intent: AddWebFeedIntent, with completion: @escaping ([String]?, Error?) -> Swift.Void)

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(provideAccountNameOptionsForAddWebFeed:withCompletion:)
    optional func provideAccountNameOptions(for intent: AddWebFeedIntent) async throws -> [String]

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @available(*, renamed: "provideFolderNameOptions(for:)")
    @objc(provideFolderNameOptionsForAddWebFeed:withCompletion:)
    optional func provideFolderNameOptions(for intent: AddWebFeedIntent, with completion: @escaping ([String]?, Error?) -> Swift.Void)

    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(provideFolderNameOptionsForAddWebFeed:withCompletion:)
    optional func provideFolderNameOptions(for intent: AddWebFeedIntent) async throws -> [String]

    @available(*, renamed: "confirm(intent:)")
    @objc(confirmAddWebFeed:completion:)
    optional func confirm(intent: AddWebFeedIntent, completion: @escaping (AddWebFeedIntentResponse) -> Swift.Void)

    /*!
     @abstract Confirmation method - Validate that this intent is ready for the next step (i.e. handling)
     @discussion Called prior to asking the app to handle the intent. The app should return a response object that contains additional information about the intent, which may be relevant for the system to show the user prior to handling. If unimplemented, the system will assume the intent is valid, and will assume there is no additional information relevant to this intent.

     @param  intent The input intent
     @param  completion The response block contains a AddWebFeedIntentResponse containing additional details about the intent that may be relevant for the system to show the user prior to handling.

     @see AddWebFeedIntentResponse
     */
    @available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
    @objc(confirmAddWebFeed:completion:)
    optional func confirm(intent: AddWebFeedIntent) async -> AddWebFeedIntentResponse

    /*!
     @abstract Default values for parameters with dynamic options
     @discussion Called to query the parameter default value.
     */
    @available(iOS 14.0, macOS 11.0, watchOS 7.0, *)
    @objc(defaultAccountNameForAddWebFeed:)
    optional func defaultAccountName(for intent: AddWebFeedIntent) -> String?

    @available(iOS 14.0, macOS 11.0, watchOS 7.0, *)
    @objc(defaultFolderNameForAddWebFeed:)
    optional func defaultFolderName(for intent: AddWebFeedIntent) -> String?

}

/*!
 @abstract Constants indicating the state of the response.
 */
@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)
@objc public enum AddWebFeedIntentResponseCode: Int {
    case unspecified = 0
    case ready
    case continueInApp
    case inProgress
    case success
    case failure
    case failureRequiringAppLaunch
}

@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)
@objc(AddWebFeedIntentResponse)
public class AddWebFeedIntentResponse: INIntentResponse {

    /*!
     @abstract The response code indicating your success or failure in confirming or handling the intent.
     */
    @objc public fileprivate(set) var code: AddWebFeedIntentResponseCode = .unspecified

    /*!
     @abstract Initializes the response object with the specified code and user activity object.
     @discussion The app extension has the option of capturing its private state as an NSUserActivity and returning it as the 'currentActivity'. If the app is launched, an NSUserActivity will be passed in with the private state. The NSUserActivity may also be used to query the app's UI extension (if provided) for a view controller representing the current intent handling state. In the case of app launch, the NSUserActivity will have its activityType set to the name of the intent. This intent object will also be available in the NSUserActivity.interaction property.

     @param  code The response code indicating your success or failure in confirming or handling the intent.
     @param  userActivity The user activity object to use when launching your app. Provide an object if you want to add information that is specific to your app. If you specify nil, the system automatically creates a user activity object for you, sets its type to the class name of the intent being handled, and fills it with an INInteraction object containing the intent and your response.
     */
    @objc(initWithCode:userActivity:)
    public convenience init(code: AddWebFeedIntentResponseCode, userActivity: NSUserActivity?) {
        self.init()
        self.code = code
        self.userActivity = userActivity
    }

}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc public enum AddWebFeedUrlUnsupportedReason: Int {
    case required = 1
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc(AddWebFeedUrlResolutionResult)
public class AddWebFeedUrlResolutionResult: INURLResolutionResult {
    @objc(unsupportedForReason:)
    public class func unsupported(forReason reason: AddWebFeedUrlUnsupportedReason) -> Self {
        return __unsupported(withReason: reason.rawValue)
    }
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc public enum AddWebFeedAccountNameUnsupportedReason: Int {
    case invalid = 1
    case communication
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc(AddWebFeedAccountNameResolutionResult)
public class AddWebFeedAccountNameResolutionResult: INStringResolutionResult {
    @objc(unsupportedForReason:)
    public class func unsupported(forReason reason: AddWebFeedAccountNameUnsupportedReason) -> Self {
        return __unsupported(withReason: reason.rawValue)
    }
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc public enum AddWebFeedFolderNameUnsupportedReason: Int {
    case invalid = 1
    case communication
}

@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)
@objc(AddWebFeedFolderNameResolutionResult)
public class AddWebFeedFolderNameResolutionResult: INStringResolutionResult {
    @objc(unsupportedForReason:)
    public class func unsupported(forReason reason: AddWebFeedFolderNameUnsupportedReason) -> Self {
        return __unsupported(withReason: reason.rawValue)
    }
}

#endif
