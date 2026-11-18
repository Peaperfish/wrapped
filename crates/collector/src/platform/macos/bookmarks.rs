//! Folder bookmarks (`NSURL` bookmark data) for the AI sources the user adds:
//! a bookmark follows a folder that was moved or renamed on the same volume,
//! so the source is found again at its new path. Not security-scoped (the app
//! is not sandboxed). Windows has nothing like it: sources there are path only.

use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_foundation::{
    NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
};
use wrapped_core::collector::ResolvedBookmark;

/// Bookmark data for an existing folder.
pub fn create(path: &Path) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions(0),
                None,
                None,
            )
            .ok()?;
        Some(data.to_vec())
    })
}

/// Where a bookmark points now, without any UI and without mounting volumes.
pub fn resolve(bytes: &[u8]) -> Option<ResolvedBookmark> {
    autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let mut stale = objc2::runtime::Bool::NO;
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithoutUI
                    | NSURLBookmarkResolutionOptions::WithoutMounting,
                None,
                &mut stale,
            )
        }
        .ok()?;
        let path = PathBuf::from(url.path()?.to_string());
        let refreshed = stale.as_bool().then(|| create(&path)).flatten();
        Some(ResolvedBookmark { path, refreshed })
    })
}
