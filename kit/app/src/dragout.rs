//! Dragging the finished vector out of the window onto the desktop, into a
//! folder or into another program, the way the original application allowed.
//! Windows only: an OLE drag offering the shell's file-list format (`CF_HDROP`)
//! for a file written beforehand into a staging folder under the user's temp
//! directory. The receiver copies that file; the staged copy is left for the
//! receiver to finish with and is tidied away on a later start.
//!
//! Like every OLE drag, the call blocks the window thread until the user drops
//! or cancels. The system pumps our messages meanwhile, winit buffers them and
//! the window keeps showing its last frame. The mouse release goes to the drag,
//! so a synthetic release is posted afterwards for the window to end the click
//! that started the drag.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutcome {
    /// The receiver accepted the file (it copies or opens it on its own).
    Dropped,
    /// Escape, a release over nothing, or a receiver that declined.
    Cancelled,
}

/// Where files offered for dragging are staged. Created on demand.
pub fn staging_dir() -> PathBuf {
    std::env::temp_dir().join("VectorMagik")
}

/// The path to stage a file under, creating the folder if needed.
pub fn staging_path(file_name: &str) -> Result<PathBuf, String> {
    let dir = staging_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot prepare the drag folder {}: {e}", dir.display()))?;
    Ok(dir.join(file_name))
}

/// Remove staged files older than a day; receivers have long finished with
/// them. Never touches anything outside the staging folder.
pub fn tidy() {
    let Ok(entries) = std::fs::read_dir(staging_dir()) else {
        return;
    };
    let old = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > old);
        if stale && entry.path().is_file() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether a dropped path is one of our own staged files: a drag that ends
/// back over the window must not be taken as a request to open it.
pub fn is_staged(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| same_dir(parent, &staging_dir()))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The shell's `DROPFILES` block: a 20-byte header (offset of the list, drop
/// point, non-client flag, wide-character flag), then every path as UTF-16
/// with a terminating zero, then one more zero closing the list.
pub fn hdrop_payload(paths: &[&Path]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&20u32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    for path in paths {
        for unit in wide(path) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}
#[cfg(not(windows))]
fn wide(path: &Path) -> Vec<u16> {
    path.to_string_lossy().encode_utf16().collect()
}

/// Offer `path` to whatever the user drops it on. Must be called on the window
/// thread while the mouse button that started the drag is still down.
#[cfg(windows)]
pub fn drag_file(path: &Path) -> Result<DragOutcome, String> {
    win::drag(path)
}
#[cfg(not(windows))]
pub fn drag_file(path: &Path) -> Result<DragOutcome, String> {
    let _ = path;
    Err("Dragging files out of the window needs Windows".into())
}

#[cfg(windows)]
mod win {
    use super::{hdrop_payload, DragOutcome};
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicU32, Ordering};

    type Hresult = i32;
    type Bool = i32;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    const fn ole_guid(data1: u32) -> Guid {
        Guid {
            data1,
            data2: 0,
            data3: 0,
            data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
        }
    }
    const IID_IUNKNOWN: Guid = ole_guid(0x0000_0000);
    const IID_IDATAOBJECT: Guid = ole_guid(0x0000_010E);
    const IID_IDROPSOURCE: Guid = ole_guid(0x0000_0121);

    const S_OK: Hresult = 0;
    const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
    const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
    const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
    const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
    const DV_E_FORMATETC: Hresult = 0x8004_0064_u32 as i32;
    const DATA_S_SAMEFORMATETC: Hresult = 0x0004_0130;
    const OLE_E_ADVISENOTSUPPORTED: Hresult = 0x8004_0003_u32 as i32;
    const DRAGDROP_S_DROP: Hresult = 0x0004_0100;
    const DRAGDROP_S_CANCEL: Hresult = 0x0004_0101;
    const DRAGDROP_S_USEDEFAULTCURSORS: Hresult = 0x0004_0102;
    const CF_HDROP: u16 = 15;
    const DVASPECT_CONTENT: u32 = 1;
    const TYMED_NULL: u32 = 0;
    const TYMED_HGLOBAL: u32 = 1;
    const DATADIR_GET: u32 = 1;
    const DROPEFFECT_NONE: u32 = 0;
    const DROPEFFECT_COPY: u32 = 1;
    const MK_LBUTTON: u32 = 0x0001;
    const GMEM_MOVEABLE: u32 = 0x0002;
    const GMEM_ZEROINIT: u32 = 0x0040;
    const WM_LBUTTONUP: u32 = 0x0202;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FormatEtc {
        cf_format: u16,
        ptd: *mut c_void,
        dw_aspect: u32,
        lindex: i32,
        tymed: u32,
    }
    #[repr(C)]
    struct StgMedium {
        tymed: u32,
        handle: *mut c_void,
        unk_for_release: *mut c_void,
    }
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    const HDROP_FORMAT: FormatEtc = FormatEtc {
        cf_format: CF_HDROP,
        ptd: null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };

    /// `IDataObject`, in vtable order.
    #[repr(C)]
    struct DataObjectVtbl {
        query_interface:
            unsafe extern "system" fn(*mut DataObject, *const Guid, *mut *mut c_void) -> Hresult,
        add_ref: unsafe extern "system" fn(*mut DataObject) -> u32,
        release: unsafe extern "system" fn(*mut DataObject) -> u32,
        get_data:
            unsafe extern "system" fn(*mut DataObject, *const FormatEtc, *mut StgMedium) -> Hresult,
        get_data_here:
            unsafe extern "system" fn(*mut DataObject, *const FormatEtc, *mut StgMedium) -> Hresult,
        query_get_data: unsafe extern "system" fn(*mut DataObject, *const FormatEtc) -> Hresult,
        get_canonical_format_etc:
            unsafe extern "system" fn(*mut DataObject, *const FormatEtc, *mut FormatEtc) -> Hresult,
        set_data: unsafe extern "system" fn(
            *mut DataObject,
            *const FormatEtc,
            *mut StgMedium,
            Bool,
        ) -> Hresult,
        enum_format_etc:
            unsafe extern "system" fn(*mut DataObject, u32, *mut *mut c_void) -> Hresult,
        d_advise: unsafe extern "system" fn(
            *mut DataObject,
            *const FormatEtc,
            u32,
            *mut c_void,
            *mut u32,
        ) -> Hresult,
        d_unadvise: unsafe extern "system" fn(*mut DataObject, u32) -> Hresult,
        enum_d_advise: unsafe extern "system" fn(*mut DataObject, *mut *mut c_void) -> Hresult,
    }
    /// The `DROPFILES` block is held as raw parts so the whole object is a
    /// plain C layout; `data_release` turns them back into the vector to free.
    #[repr(C)]
    struct DataObject {
        vtbl: *const DataObjectVtbl,
        refs: AtomicU32,
        payload: *mut u8,
        len: usize,
        capacity: usize,
    }

    /// `IDropSource`, in vtable order.
    #[repr(C)]
    struct DropSourceVtbl {
        query_interface:
            unsafe extern "system" fn(*mut DropSource, *const Guid, *mut *mut c_void) -> Hresult,
        add_ref: unsafe extern "system" fn(*mut DropSource) -> u32,
        release: unsafe extern "system" fn(*mut DropSource) -> u32,
        query_continue_drag: unsafe extern "system" fn(*mut DropSource, Bool, u32) -> Hresult,
        give_feedback: unsafe extern "system" fn(*mut DropSource, u32) -> Hresult,
    }
    #[repr(C)]
    struct DropSource {
        vtbl: *const DropSourceVtbl,
        refs: AtomicU32,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn OleInitialize(reserved: *mut c_void) -> Hresult;
        fn DoDragDrop(
            data: *mut DataObject,
            source: *mut DropSource,
            ok_effects: u32,
            effect: *mut u32,
        ) -> Hresult;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHCreateStdEnumFmtEtc(
            count: u32,
            formats: *const FormatEtc,
            out: *mut *mut c_void,
        ) -> Hresult;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(handle: *mut c_void) -> Bool;
        fn GlobalFree(handle: *mut c_void) -> *mut c_void;
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> isize;
        fn GetForegroundWindow() -> isize;
        fn GetCursorPos(point: *mut Point) -> Bool;
        fn ScreenToClient(window: isize, point: *mut Point) -> Bool;
        fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> Bool;
    }

    unsafe fn same_guid(a: *const Guid, b: &Guid) -> bool {
        !a.is_null() && {
            let a = &*a;
            a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
        }
    }
    fn accepts(format: &FormatEtc) -> bool {
        format.cf_format == CF_HDROP
            && format.tymed & TYMED_HGLOBAL != 0
            && format.dw_aspect == DVASPECT_CONTENT
    }

    unsafe extern "system" fn data_query_interface(
        this: *mut DataObject,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> Hresult {
        if out.is_null() {
            return E_POINTER;
        }
        if same_guid(iid, &IID_IUNKNOWN) || same_guid(iid, &IID_IDATAOBJECT) {
            data_add_ref(this);
            *out = this as *mut c_void;
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
    unsafe extern "system" fn data_add_ref(this: *mut DataObject) -> u32 {
        (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
    }
    unsafe extern "system" fn data_release(this: *mut DataObject) -> u32 {
        let left = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            let object = Box::from_raw(this);
            drop(Vec::from_raw_parts(
                object.payload,
                object.len,
                object.capacity,
            ));
            drop(object);
        }
        left
    }
    unsafe extern "system" fn data_get_data(
        this: *mut DataObject,
        format: *const FormatEtc,
        medium: *mut StgMedium,
    ) -> Hresult {
        if format.is_null() || medium.is_null() {
            return E_POINTER;
        }
        // Conventional hygiene: a failed request leaves an empty medium.
        (*medium).tymed = TYMED_NULL;
        (*medium).handle = null_mut();
        (*medium).unk_for_release = null_mut();
        if !accepts(&*format) {
            return DV_E_FORMATETC;
        }
        // The receiver owns the returned block and frees it with
        // ReleaseStgMedium, so every request gets its own copy.
        let (payload, len) = ((*this).payload, (*this).len);
        let handle = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, len);
        if handle.is_null() {
            return E_OUTOFMEMORY;
        }
        let block = GlobalLock(handle);
        if block.is_null() {
            GlobalFree(handle);
            return E_OUTOFMEMORY;
        }
        std::ptr::copy_nonoverlapping(payload, block as *mut u8, len);
        GlobalUnlock(handle);
        (*medium).tymed = TYMED_HGLOBAL;
        (*medium).handle = handle;
        (*medium).unk_for_release = null_mut();
        S_OK
    }
    unsafe extern "system" fn data_get_data_here(
        _: *mut DataObject,
        _: *const FormatEtc,
        _: *mut StgMedium,
    ) -> Hresult {
        E_NOTIMPL
    }
    unsafe extern "system" fn data_query_get_data(
        _: *mut DataObject,
        format: *const FormatEtc,
    ) -> Hresult {
        if format.is_null() {
            E_POINTER
        } else if accepts(&*format) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    unsafe extern "system" fn data_get_canonical_format_etc(
        _: *mut DataObject,
        format: *const FormatEtc,
        out: *mut FormatEtc,
    ) -> Hresult {
        if format.is_null() || out.is_null() {
            return E_POINTER;
        }
        *out = *format;
        (*out).ptd = null_mut();
        DATA_S_SAMEFORMATETC
    }
    unsafe extern "system" fn data_set_data(
        _: *mut DataObject,
        _: *const FormatEtc,
        _: *mut StgMedium,
        _: Bool,
    ) -> Hresult {
        E_NOTIMPL
    }
    unsafe extern "system" fn data_enum_format_etc(
        _: *mut DataObject,
        direction: u32,
        out: *mut *mut c_void,
    ) -> Hresult {
        if out.is_null() {
            return E_POINTER;
        }
        if direction == DATADIR_GET {
            SHCreateStdEnumFmtEtc(1, &HDROP_FORMAT, out)
        } else {
            *out = null_mut();
            E_NOTIMPL
        }
    }
    unsafe extern "system" fn data_d_advise(
        _: *mut DataObject,
        _: *const FormatEtc,
        _: u32,
        _: *mut c_void,
        _: *mut u32,
    ) -> Hresult {
        OLE_E_ADVISENOTSUPPORTED
    }
    unsafe extern "system" fn data_d_unadvise(_: *mut DataObject, _: u32) -> Hresult {
        OLE_E_ADVISENOTSUPPORTED
    }
    unsafe extern "system" fn data_enum_d_advise(
        _: *mut DataObject,
        out: *mut *mut c_void,
    ) -> Hresult {
        if !out.is_null() {
            *out = null_mut();
        }
        OLE_E_ADVISENOTSUPPORTED
    }
    static DATA_VTBL: DataObjectVtbl = DataObjectVtbl {
        query_interface: data_query_interface,
        add_ref: data_add_ref,
        release: data_release,
        get_data: data_get_data,
        get_data_here: data_get_data_here,
        query_get_data: data_query_get_data,
        get_canonical_format_etc: data_get_canonical_format_etc,
        set_data: data_set_data,
        enum_format_etc: data_enum_format_etc,
        d_advise: data_d_advise,
        d_unadvise: data_d_unadvise,
        enum_d_advise: data_enum_d_advise,
    };

    unsafe extern "system" fn source_query_interface(
        this: *mut DropSource,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> Hresult {
        if out.is_null() {
            return E_POINTER;
        }
        if same_guid(iid, &IID_IUNKNOWN) || same_guid(iid, &IID_IDROPSOURCE) {
            source_add_ref(this);
            *out = this as *mut c_void;
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
    unsafe extern "system" fn source_add_ref(this: *mut DropSource) -> u32 {
        (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
    }
    unsafe extern "system" fn source_release(this: *mut DropSource) -> u32 {
        let left = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(this));
        }
        left
    }
    /// Escape cancels; releasing the button drops; anything else continues.
    unsafe extern "system" fn source_query_continue_drag(
        _: *mut DropSource,
        escape_pressed: Bool,
        key_state: u32,
    ) -> Hresult {
        if escape_pressed != 0 {
            DRAGDROP_S_CANCEL
        } else if key_state & MK_LBUTTON == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    unsafe extern "system" fn source_give_feedback(_: *mut DropSource, _: u32) -> Hresult {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
    static SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
        query_interface: source_query_interface,
        add_ref: source_add_ref,
        release: source_release,
        query_continue_drag: source_query_continue_drag,
        give_feedback: source_give_feedback,
    };

    thread_local! {
        static OLE_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub fn drag(path: &Path) -> Result<DragOutcome, String> {
        let window = unsafe {
            let active = GetActiveWindow();
            if active != 0 {
                active
            } else {
                GetForegroundWindow()
            }
        };
        // winit initialises OLE on the window thread for its own drop target;
        // a repeated initialisation just reports that it was done already.
        // Never uninitialised: the thread keeps OLE until the process ends.
        let ready = OLE_READY.with(|ready| {
            if !ready.get() {
                let code = unsafe { OleInitialize(null_mut()) };
                ready.set(code >= 0);
            }
            ready.get()
        });
        if !ready {
            return Err("Drag and drop is not available on this window thread".into());
        }
        let mut payload = std::mem::ManuallyDrop::new(hdrop_payload(&[path]));
        let data = Box::into_raw(Box::new(DataObject {
            vtbl: &DATA_VTBL,
            refs: AtomicU32::new(1),
            payload: payload.as_mut_ptr(),
            len: payload.len(),
            capacity: payload.capacity(),
        }));
        let source = Box::into_raw(Box::new(DropSource {
            vtbl: &SOURCE_VTBL,
            refs: AtomicU32::new(1),
        }));
        let mut effect = DROPEFFECT_NONE;
        let code = unsafe { DoDragDrop(data, source, DROPEFFECT_COPY, &mut effect) };
        // The receiver may keep the data object after the drop; our reference
        // goes and the last holder frees it.
        unsafe {
            data_release(data);
            source_release(source);
            release_click(window);
        }
        match code {
            DRAGDROP_S_DROP if effect != DROPEFFECT_NONE => Ok(DragOutcome::Dropped),
            DRAGDROP_S_DROP | DRAGDROP_S_CANCEL => Ok(DragOutcome::Cancelled),
            other => Err(format!(
                "The drag could not start (error 0x{:08X})",
                other as u32
            )),
        }
    }

    /// The button release that ended the drag went to the drag's own window;
    /// hand the window one at the pointer so its click state matches reality.
    unsafe fn release_click(window: isize) {
        if window == 0 {
            return;
        }
        let mut at = Point { x: 0, y: 0 };
        if GetCursorPos(&mut at) == 0 {
            return;
        }
        ScreenToClient(window, &mut at);
        let lparam = ((at.y as u32 & 0xFFFF) << 16 | (at.x as u32 & 0xFFFF)) as i32 as isize;
        PostMessageW(window, WM_LBUTTONUP, 0, lparam);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_a_dropfiles_block_with_wide_paths() {
        let bytes = hdrop_payload(&[Path::new("C:\\a\\b.svg"), Path::new("D:\\c.pdf")]);
        assert_eq!(
            &bytes[..4],
            &20u32.to_le_bytes(),
            "list starts after the header"
        );
        assert_eq!(&bytes[4..16], &[0u8; 12], "drop point and client flag");
        assert_eq!(&bytes[16..20], &1u32.to_le_bytes(), "wide characters");
        let wide: Vec<u16> = bytes[20..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&wide).unwrap(),
            "C:\\a\\b.svg\0D:\\c.pdf\0\0"
        );
        assert_eq!(bytes.len() % 2, 0);
    }

    #[test]
    fn staged_paths_are_recognised_and_stale_ones_tidied() {
        let staged = staging_path("unit-test.svg").unwrap();
        std::fs::write(&staged, "<svg/>").unwrap();
        assert!(is_staged(&staged));
        assert!(!is_staged(Path::new("C:\\somewhere\\else.svg")));
        tidy();
        assert!(staged.exists(), "a fresh file survives tidying");
        std::fs::remove_file(&staged).unwrap();
    }
}
