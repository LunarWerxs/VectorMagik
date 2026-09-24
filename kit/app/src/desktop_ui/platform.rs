//! Part of `desktop_ui`: what differs between the Windows window and the
//! same app in a browser tab (`kit/web`). The window runs conversions,
//! derivations and tiles on threads of their own; a browser tab has one
//! thread, so there the work waits for the start of the next frame and runs
//! then, after the frame that shows it started has been painted. Files, the
//! preferences and the system dialogs go through the page instead of the
//! disk: the tab asks the page for a file chooser, downloads what it saves
//! and keeps its preferences in the page's storage.

use std::sync::mpsc::{self, Receiver, Sender};

/// Whether a page hosts the app (`kit/web`) rather than a window.
pub const IN_BROWSER: bool = !cfg!(feature = "desktop");

/// How long the view must rest before a crisp tile is asked for. The window
/// asks at once: its render thread keeps only the latest request. A tab
/// renders a tile before painting the next frame, so it asks once a zoom or
/// a pan has stopped and a gesture renders one tile, not one per step.
pub(super) const TILE_REST: std::time::Duration = if IN_BROWSER {
    std::time::Duration::from_millis(150)
} else {
    std::time::Duration::ZERO
};

/// Run `job` off the UI thread.
pub(super) fn spawn(job: impl FnOnce() + Send + 'static) {
    #[cfg(feature = "desktop")]
    std::thread::spawn(job);
    #[cfg(not(feature = "desktop"))]
    browser::JOBS.with(|jobs| jobs.borrow_mut().push(Box::new(job)));
}

/// Answer requests off the UI thread, always the latest one first: requests
/// sent while one is being answered replace each other, so dragging a slider
/// never queues up stale work. `answer` may give nothing back for a request.
pub(super) fn serve<Q: Send + 'static, R: Send + 'static>(
    ctx: egui::Context,
    mut answer: impl FnMut(Q) -> Option<R> + Send + 'static,
) -> (Sender<Q>, Receiver<R>) {
    let (request_tx, request_rx) = mpsc::channel::<Q>();
    let (result_tx, result_rx) = mpsc::channel::<R>();
    #[cfg(feature = "desktop")]
    std::thread::spawn(move || {
        while let Ok(mut request) = request_rx.recv() {
            while let Ok(newer) = request_rx.try_recv() {
                request = newer;
            }
            if let Some(result) = answer(request) {
                if result_tx.send(result).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        }
    });
    #[cfg(not(feature = "desktop"))]
    {
        let mut latest = None;
        browser::SERVICES.with(|services| {
            services.borrow_mut().push(Box::new(move |now: bool| {
                loop {
                    match request_rx.try_recv() {
                        Ok(request) => latest = Some(request),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return browser::Served::Gone,
                    }
                }
                if now {
                    if let Some(result) = latest.take().and_then(&mut answer) {
                        if result_tx.send(result).is_err() {
                            return browser::Served::Gone;
                        }
                        ctx.request_repaint();
                    }
                }
                if latest.is_some() {
                    browser::Served::Waiting
                } else {
                    browser::Served::Idle
                }
            }));
        });
    }
    (request_tx, result_rx)
}

/// In a browser tab: run the work `spawn` and `serve` were given since the
/// last frame. The page calls it before every frame; the window has nothing
/// waiting.
pub fn run_pending() {
    #[cfg(not(feature = "desktop"))]
    browser::run_pending();
}

/// Whether work is waiting for the next frame, so the page runs one soon.
pub fn has_pending() -> bool {
    #[cfg(not(feature = "desktop"))]
    {
        browser::has_pending()
    }
    #[cfg(feature = "desktop")]
    false
}

/// What the tab asks of the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Show the file chooser; the picked file comes back as a dropped file.
    OpenPicker,
    /// Save `data` as the file `name`.
    Download {
        name: String,
        mime: &'static str,
        data: Vec<u8>,
    },
    /// Keep the preferences for the next visit.
    StorePrefs(String),
    /// Read the canvas once this frame is painted and send it back.
    Screenshot,
}

/// Ask the page for something (in the window, nothing is asked).
pub(super) fn ask(command: Command) {
    #[cfg(not(feature = "desktop"))]
    browser::COMMANDS.with(|commands| commands.borrow_mut().push(command));
    #[cfg(feature = "desktop")]
    let _ = command;
}

/// What the tab asked of the page since the last call.
pub fn take_commands() -> Vec<Command> {
    #[cfg(not(feature = "desktop"))]
    {
        browser::COMMANDS.with(|commands| std::mem::take(&mut *commands.borrow_mut()))
    }
    #[cfg(feature = "desktop")]
    Vec::new()
}

#[cfg(not(feature = "desktop"))]
mod browser {
    use super::Command;
    use std::cell::RefCell;

    type Job = Box<dyn FnOnce()>;
    /// A standing server, told whether to answer its latest request now or
    /// only to take in what was sent.
    type Service = Box<dyn FnMut(bool) -> Served>;

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Served {
        Idle,
        /// A request waits for the next frame.
        Waiting,
        /// Nobody sends or listens any more.
        Gone,
    }

    thread_local! {
        pub(super) static JOBS: RefCell<Vec<Job>> = const { RefCell::new(Vec::new()) };
        pub(super) static SERVICES: RefCell<Vec<Service>> = const { RefCell::new(Vec::new()) };
        pub(super) static COMMANDS: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    }

    /// Call every server, keeping those still in use; taken out of the list
    /// while they run, so one may start another server meanwhile.
    fn each_service(now: bool) -> bool {
        let mut services = SERVICES.with(|services| std::mem::take(&mut *services.borrow_mut()));
        let mut waiting = false;
        services.retain_mut(|service| match service(now) {
            Served::Gone => false,
            Served::Waiting => {
                waiting = true;
                true
            }
            Served::Idle => true,
        });
        SERVICES.with(|list| {
            let mut list = list.borrow_mut();
            services.append(&mut list);
            *list = services;
        });
        waiting
    }

    pub(super) fn run_pending() {
        let jobs = JOBS.with(|jobs| std::mem::take(&mut *jobs.borrow_mut()));
        for job in jobs {
            job();
        }
        each_service(true);
    }

    pub(super) fn has_pending() -> bool {
        let jobs = JOBS.with(|jobs| !jobs.borrow().is_empty());
        each_service(false) || jobs
    }
}
