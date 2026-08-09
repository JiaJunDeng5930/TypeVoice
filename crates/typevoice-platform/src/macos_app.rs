use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrontmostApplication {
    pub pid: i32,
    pub name: Option<String>,
    pub bundle_identifier: Option<String>,
}

pub(crate) fn frontmost_application() -> Option<FrontmostApplication> {
    let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(FrontmostApplication {
        pid: application.processIdentifier(),
        name: application.localizedName().map(|value| value.to_string()),
        bundle_identifier: application
            .bundleIdentifier()
            .map(|value| value.to_string()),
    })
}

pub(crate) fn activate_application(pid: i32) -> bool {
    let Some(application) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
    else {
        return false;
    };
    application.activateWithOptions(NSApplicationActivationOptions::empty())
}
