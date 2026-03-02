use std::process::Command;

pub trait CommandNoConsoleExt {
    fn no_console(&mut self) -> &mut Self;
}

impl CommandNoConsoleExt for Command {
    fn no_console(&mut self) -> &mut Self {
        #[cfg(all(windows, not(debug_assertions)))]
        {
            use std::os::windows::process::CommandExt as _;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
