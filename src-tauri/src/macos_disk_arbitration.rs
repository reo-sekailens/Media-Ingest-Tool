//! Disk Arbitration lifecycle notifications for macOS removable-media discovery.
//!
//! Callbacks deliberately carry only a coalesced reconciliation request. Disk
//! Arbitration callback order is not a durable device state, so the consumer
//! must perform a fresh authoritative discovery snapshot before acknowledging
//! an event. No shell command is invoked from this bridge.

#![cfg(target_os = "macos")]

use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

/// A signal to take a fresh authoritative Disk Arbitration/device snapshot.
///
/// `generation` advances for every native callback coalesced into the signal.
/// It is not a media identity or connection generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskArbitrationReconcileRequest {
    pub generation: u64,
}

/// Owns the dedicated Disk Arbitration run-loop thread.
///
/// After processing a request, call [`Self::acknowledge`] with its generation.
/// If an event arrived while the snapshot was being reconciled, acknowledgement
/// queues a single follow-up request rather than losing that event.
pub struct DiskArbitrationSubscription {
    state: Arc<SubscriptionState>,
    worker: Option<JoinHandle<()>>,
}

impl DiskArbitrationSubscription {
    /// Marks a completed snapshot reconciliation.
    ///
    /// The acknowledgement must happen only after the consumer has read a
    /// fresh device snapshot. Passing an older request generation is safe: a
    /// newer coalesced request remains queued.
    pub fn acknowledge(&self, request: DiskArbitrationReconcileRequest) {
        self.state.acknowledge(request.generation);
    }
}

impl Drop for DiskArbitrationSubscription {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        let run_loop = self.state.run_loop.load(Ordering::Acquire);
        if run_loop != 0 {
            // CFRunLoopStop is thread-safe and wakes a currently sleeping
            // run-loop so the worker can unschedule its Disk Arbitration
            // session before the callback context is released.
            unsafe {
                CFRunLoopStop(run_loop as CFRunLoopRef);
                CFRunLoopWakeUp(run_loop as CFRunLoopRef);
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Starts the native Disk Arbitration listener before taking an initial
/// snapshot. `queue_capacity` must be non-zero; one outstanding request is
/// enough because subsequent callbacks are coalesced until acknowledgement.
pub fn subscribe(
    queue_capacity: usize,
) -> Result<
    (
        DiskArbitrationSubscription,
        Receiver<DiskArbitrationReconcileRequest>,
    ),
    DiskArbitrationError,
> {
    if queue_capacity == 0 {
        return Err(DiskArbitrationError::ZeroCapacity);
    }

    let (sender, receiver) = mpsc::sync_channel(queue_capacity);
    let state = Arc::new(SubscriptionState::new(sender));
    let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
    let worker_state = Arc::clone(&state);
    let worker = thread::Builder::new()
        .name("media-ingest-disk-arbitration".into())
        .spawn(move || run_session(worker_state, startup_sender))
        .map_err(DiskArbitrationError::ThreadStart)?;

    match startup_receiver.recv() {
        Ok(Ok(())) => Ok((
            DiskArbitrationSubscription {
                state,
                worker: Some(worker),
            },
            receiver,
        )),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        }
        Err(_) => {
            let _ = worker.join();
            Err(DiskArbitrationError::StartupChannelClosed)
        }
    }
}

#[derive(Debug)]
pub enum DiskArbitrationError {
    ZeroCapacity,
    ThreadStart(std::io::Error),
    SessionCreationFailed,
    StartupChannelClosed,
}

impl std::fmt::Display for DiskArbitrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCapacity => {
                formatter.write_str("Disk Arbitration queue capacity must be non-zero")
            }
            Self::ThreadStart(error) => write!(
                formatter,
                "could not start Disk Arbitration worker: {error}"
            ),
            Self::SessionCreationFailed => {
                formatter.write_str("could not create Disk Arbitration session")
            }
            Self::StartupChannelClosed => {
                formatter.write_str("Disk Arbitration worker stopped before startup")
            }
        }
    }
}

impl std::error::Error for DiskArbitrationError {}

struct CoalescingState {
    generation: u64,
    pending: bool,
}

struct SubscriptionState {
    sender: SyncSender<DiskArbitrationReconcileRequest>,
    coalescing: Mutex<CoalescingState>,
    stopped: AtomicBool,
    run_loop: AtomicUsize,
}

impl SubscriptionState {
    fn new(sender: SyncSender<DiskArbitrationReconcileRequest>) -> Self {
        Self {
            sender,
            coalescing: Mutex::new(CoalescingState {
                generation: 0,
                pending: false,
            }),
            stopped: AtomicBool::new(false),
            run_loop: AtomicUsize::new(0),
        }
    }

    fn request_reconciliation(&self) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut coalescing = self.coalescing.lock().expect("Disk Arbitration state lock");
        coalescing.generation = coalescing.generation.wrapping_add(1);
        if coalescing.pending {
            return;
        }
        coalescing.pending = true;
        self.send_locked(&mut coalescing);
    }

    fn acknowledge(&self, completed_generation: u64) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut coalescing = self.coalescing.lock().expect("Disk Arbitration state lock");
        if coalescing.generation <= completed_generation {
            coalescing.pending = false;
            return;
        }

        // A callback occurred after the request was emitted. The receiver has
        // consumed that request, so enqueue exactly one follow-up snapshot.
        coalescing.pending = true;
        self.send_locked(&mut coalescing);
    }

    fn send_locked(&self, coalescing: &mut CoalescingState) {
        let request = DiskArbitrationReconcileRequest {
            generation: coalescing.generation,
        };
        match self.sender.try_send(request) {
            Ok(()) => {}
            // A listener owns the only sender and keeps `pending` true until
            // acknowledgement. With a non-zero queue this cannot occur during
            // normal operation, but retaining pending is safer than emitting a
            // storm if a future integration changes that invariant.
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                coalescing.pending = true;
            }
        }
    }
}

fn run_session(
    state: Arc<SubscriptionState>,
    startup: SyncSender<Result<(), DiskArbitrationError>>,
) {
    unsafe {
        let session = DASessionCreate(ptr::null());
        if session.is_null() {
            let _ = startup.send(Err(DiskArbitrationError::SessionCreationFailed));
            return;
        }

        let run_loop = CFRunLoopGetCurrent();
        state.run_loop.store(run_loop as usize, Ordering::Release);
        let context = Arc::as_ptr(&state).cast_mut().cast::<c_void>();
        DARegisterDiskAppearedCallback(session, ptr::null(), disk_appeared, context);
        DARegisterDiskDescriptionChangedCallback(
            session,
            ptr::null(),
            disk_description_changed,
            context,
        );
        DARegisterDiskDisappearedCallback(session, ptr::null(), disk_disappeared, context);
        DASessionScheduleWithRunLoop(session, run_loop, kCFRunLoopDefaultMode);

        if startup.send(Ok(())).is_err() {
            DASessionUnscheduleFromRunLoop(session, run_loop, kCFRunLoopDefaultMode);
            state.run_loop.store(0, Ordering::Release);
            CFRelease(session);
            return;
        }

        while !state.stopped.load(Ordering::Acquire) {
            // One-second timeout is only a shutdown safety net. Native events,
            // rather than polling, drive reconciliation.
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0, true);
        }

        DASessionUnscheduleFromRunLoop(session, run_loop, kCFRunLoopDefaultMode);
        state.run_loop.store(0, Ordering::Release);
        CFRelease(session);
    }
}

unsafe extern "C" fn disk_appeared(_disk: DADiskRef, context: *mut c_void) {
    request_from_callback(context);
}

unsafe extern "C" fn disk_description_changed(
    _disk: DADiskRef,
    _changed_keys: CFArrayRef,
    context: *mut c_void,
) {
    request_from_callback(context);
}

unsafe extern "C" fn disk_disappeared(_disk: DADiskRef, context: *mut c_void) {
    request_from_callback(context);
}

unsafe fn request_from_callback(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // The worker unschedules the session before its `Arc` can be released.
    let state = &*context.cast::<SubscriptionState>();
    state.request_reconciliation();
}

type DASessionRef = *mut c_void;
type DADiskRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;

#[link(name = "DiskArbitration", kind = "framework")]
extern "C" {
    fn DASessionCreate(allocator: CFAllocatorRef) -> DASessionRef;
    fn DASessionScheduleWithRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn DASessionUnscheduleFromRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn DARegisterDiskAppearedCallback(
        session: DASessionRef,
        matching: CFDictionaryRef,
        callback: unsafe extern "C" fn(DADiskRef, *mut c_void),
        context: *mut c_void,
    );
    fn DARegisterDiskDescriptionChangedCallback(
        session: DASessionRef,
        matching: CFDictionaryRef,
        callback: unsafe extern "C" fn(DADiskRef, CFArrayRef, *mut c_void),
        context: *mut c_void,
    );
    fn DARegisterDiskDisappearedCallback(
        session: DASessionRef,
        matching: CFDictionaryRef,
        callback: unsafe extern "C" fn(DADiskRef, *mut c_void),
        context: *mut c_void,
    );
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRelease(object: *const c_void);
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopRunInMode(
        mode: CFStringRef,
        seconds: f64,
        return_after_source_handled: bool,
    ) -> i32;
    fn CFRunLoopStop(run_loop: CFRunLoopRef);
    fn CFRunLoopWakeUp(run_loop: CFRunLoopRef);
}
