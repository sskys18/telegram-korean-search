import Foundation

public enum WikiLanguage: String {
    case en
    case ko

    public static func systemDefault() -> WikiLanguage {
        if let code = Locale.current.language.languageCode?.identifier,
           code == "ko" {
            return .ko
        }
        return .en
    }
}

public enum WikiLocale {
    private static let key = "seoyu.wiki.language"

    public static var current: WikiLanguage {
        get {
            if let raw = UserDefaults.standard.string(forKey: key),
               let lang = WikiLanguage(rawValue: raw) {
                return lang
            }
            return .systemDefault()
        }
        set {
            UserDefaults.standard.set(newValue.rawValue, forKey: key)
            NotificationCenter.default.post(name: .seoyuWikiLanguageChanged, object: nil)
        }
    }
}

public extension Notification.Name {
    static let seoyuWikiLanguageChanged = Notification.Name("seoyu.wiki.language.changed")
    static let seoyuWikiTopicsChanged = Notification.Name("seoyu.wiki.topics.changed")
    static let seoyuWikiProgress = Notification.Name("seoyu.wiki.progress")
    static let seoyuWikiError = Notification.Name("seoyu.wiki.error")
}
