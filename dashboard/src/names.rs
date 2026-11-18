//! Display names for bundle ids (the `app_names` command).
//!
//! macOS: the installed app found through Launch Services
//! (`NSWorkspace URLForApplicationWithBundleIdentifier:`), then its bundle's
//! display name (`CFBundleDisplayName`, else `CFBundleName`, else the file
//! name without `.app`). Ids with no installed app, and keys that are not
//! bundle ids ("idle"), come back unchanged. Resolved names are cached for
//! the process. Other platforms: ids unchanged.

use std::collections::HashMap;

/// Map every id to its display name (itself when there is none).
pub fn app_names(ids: &[String]) -> HashMap<String, String> {
    ids.iter().map(|id| (id.clone(), name_of(id))).collect()
}

/// Only reverse-DNS-looking ids can be bundle ids.
fn looks_like_bundle_id(id: &str) -> bool {
    id.contains('.')
        && !id.starts_with('.')
        && !id.ends_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

#[cfg(target_os = "macos")]
fn name_of(id: &str) -> String {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    if !looks_like_bundle_id(id) {
        return id.to_string();
    }
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(id).cloned()) {
        return hit;
    }
    let name = macos::display_name(id).unwrap_or_else(|| id.to_string());
    if let Ok(mut c) = cache.lock() {
        c.insert(id.to_string(), name.clone());
    }
    name
}

#[cfg(not(target_os = "macos"))]
fn name_of(id: &str) -> String {
    let _ = looks_like_bundle_id;
    id.to_string()
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSString};

    pub fn display_name(id: &str) -> Option<String> {
        autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(id))?;
            let bundle = NSBundle::bundleWithURL(&url)?;
            for key in ["CFBundleDisplayName", "CFBundleName"] {
                let value: Option<objc2::rc::Retained<AnyObject>> =
                    bundle.objectForInfoDictionaryKey(&NSString::from_str(key));
                if let Some(s) = value.and_then(|v| v.downcast::<NSString>().ok()) {
                    let s = s.to_string();
                    if !s.trim().is_empty() {
                        return Some(s);
                    }
                }
            }
            let path = url.path()?.to_string();
            std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_bundle_keys_and_missing_apps_pass_through() {
        let ids: Vec<String> = ["idle", "(unknown)", "com.example.NoSuchAppAnywhere"]
            .map(String::from)
            .to_vec();
        let names = app_names(&ids);
        for id in &ids {
            assert_eq!(&names[id], id);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_is_found() {
        assert_eq!(name_of("com.apple.finder"), "Finder");
    }
}
