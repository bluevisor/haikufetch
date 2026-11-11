//! macOS. sysctl and libproc for the basics, IOKit for the GPU, battery and model, CoreGraphics
//! for displays, the configd scan record and CoreWLAN for the Wi-Fi name.
//!
//! Only CoreFoundation and IOKit are linked. CoreGraphics, SystemConfiguration, Foundation and
//! CoreWLAN are opened with dlopen by the module that needs them, on that module's thread, so their
//! load time overlaps with everything else instead of delaying launch.

#![allow(non_upper_case_globals, non_snake_case, clippy::upper_case_acronyms)]

use std::{
    ffi::{CStr, CString, c_char, c_void},
    mem::{MaybeUninit, size_of},
    net::Ipv6Addr,
    sync::OnceLock,
};

use crate::{
    info::*,
    net::{self, V6State},
};

// ---- CoreFoundation ------------------------------------------------------------------------

type CFTypeRef = *const c_void;
type CFIndex = isize;
const UTF8: u32 = 0x0800_0100;
const kCFNumberSInt64Type: CFIndex = 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFStringCreateWithBytes(
        alloc: CFTypeRef,
        bytes: *const u8,
        len: CFIndex,
        encoding: u32,
        external: u8,
    ) -> CFTypeRef;
    fn CFStringGetLength(s: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(s: CFTypeRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    fn CFDataGetLength(d: CFTypeRef) -> CFIndex;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
    fn CFNumberGetValue(n: CFTypeRef, kind: CFIndex, out: *mut c_void) -> u8;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryGetCount(d: CFTypeRef) -> CFIndex;
    fn CFDictionaryGetKeysAndValues(d: CFTypeRef, keys: *mut CFTypeRef, values: *mut CFTypeRef);
    fn CFDictionarySetValue(d: CFTypeRef, key: CFTypeRef, value: CFTypeRef);
}

/// An owned CF object, released on drop
struct Cf(CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: Cf only holds objects we own a reference to
        unsafe { CFRelease(self.0) }
    }
}

impl Cf {
    /// Takes ownership of a +1 reference, None for null
    fn owned(object: CFTypeRef) -> Option<Cf> {
        (!object.is_null()).then(|| Cf(object))
    }

    fn string(text: &str) -> Option<Cf> {
        // SAFETY: the bytes are valid UTF-8 for their length
        Cf::owned(unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as CFIndex,
                UTF8,
                0,
            )
        })
    }
}

fn type_is(object: CFTypeRef, id: usize) -> bool {
    // SAFETY: object is a non-null CF object
    !object.is_null() && unsafe { CFGetTypeID(object) } == id
}

/// A CFString's text, or a NUL-terminated CFData's (IOKit stores many names that way). Trimmed,
/// None when empty
fn cf_text(object: CFTypeRef) -> Option<String> {
    // SAFETY: each branch checks the type before using the matching accessor
    unsafe {
        let text = if type_is(object, CFStringGetTypeID()) {
            let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(object), UTF8) + 1;
            let mut buffer = vec![0 as c_char; size.max(1) as usize];
            if CFStringGetCString(object, buffer.as_mut_ptr(), size, UTF8) == 0 {
                return None;
            }
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        } else if type_is(object, CFDataGetTypeID()) {
            let bytes = cf_bytes(object);
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        } else {
            return None;
        };
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// A CFData's bytes (copied)
fn cf_bytes(object: CFTypeRef) -> Vec<u8> {
    if !type_is(object, unsafe { CFDataGetTypeID() }) {
        return Vec::new();
    }
    // SAFETY: a CFData's pointer is valid for its length
    unsafe {
        let length = CFDataGetLength(object).max(0) as usize;
        let pointer = CFDataGetBytePtr(object);
        if pointer.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(pointer, length).to_vec()
        }
    }
}

/// A CFNumber, or the first 4 bytes of a little-endian CFData
fn cf_i64(object: CFTypeRef) -> Option<i64> {
    // SAFETY: type checked first
    unsafe {
        if type_is(object, CFNumberGetTypeID()) {
            let mut value = 0i64;
            return (CFNumberGetValue(
                object,
                kCFNumberSInt64Type,
                &mut value as *mut i64 as *mut c_void,
            ) != 0)
                .then_some(value);
        }
        if type_is(object, CFBooleanGetTypeID()) {
            return Some(CFBooleanGetValue(object) as i64);
        }
    }
    let bytes = cf_bytes(object);
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as i64)
}

fn cf_bool(object: CFTypeRef) -> Option<bool> {
    cf_i64(object).map(|v| v != 0)
}

fn dict_get(dict: CFTypeRef, key: &str) -> CFTypeRef {
    let Some(key) = Cf::string(key) else {
        return std::ptr::null();
    };
    if !type_is(dict, unsafe { CFDictionaryGetTypeID() }) {
        return std::ptr::null();
    }
    // SAFETY: dict is a CFDictionary, the value isn't retained (get rule)
    unsafe { CFDictionaryGetValue(dict, key.0) }
}

// ---- IOKit ---------------------------------------------------------------------------------

type IoObject = u32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceNameMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingService(main: u32, matching: CFTypeRef) -> IoObject;
    fn IOServiceGetMatchingServices(main: u32, matching: CFTypeRef, iterator: *mut IoObject)
    -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CFTypeRef,
        alloc: CFTypeRef,
        options: u32,
    ) -> CFTypeRef;
    fn IORegistryEntryFromPath(main: u32, path: *const c_char) -> IoObject;
    fn IORegistryEntryGetParentEntry(
        entry: IoObject,
        plane: *const c_char,
        parent: *mut IoObject,
    ) -> i32;
    fn IOObjectConformsTo(object: IoObject, class: *const c_char) -> u32;
}

/// An owned registry entry, released on drop
struct Io(IoObject);

impl Drop for Io {
    fn drop(&mut self) {
        // SAFETY: we own this reference
        unsafe { IOObjectRelease(self.0) };
    }
}

impl Io {
    fn owned(object: IoObject) -> Option<Io> {
        (object != 0).then(|| Io(object))
    }

    fn from_path(path: &CStr) -> Option<Io> {
        // SAFETY: a valid C path
        Io::owned(unsafe { IORegistryEntryFromPath(0, path.as_ptr()) })
    }

    fn property(&self, key: &str) -> Option<Cf> {
        let key = Cf::string(key)?;
        // SAFETY: create rule, we own the result
        Cf::owned(unsafe { IORegistryEntryCreateCFProperty(self.0, key.0, std::ptr::null(), 0) })
    }

    fn conforms(&self, class: &CStr) -> bool {
        // SAFETY: a valid entry and class name
        unsafe { IOObjectConformsTo(self.0, class.as_ptr()) != 0 }
    }

    fn parent(&self) -> Option<Io> {
        let mut parent = 0;
        // SAFETY: on success parent is a +1 reference
        (unsafe { IORegistryEntryGetParentEntry(self.0, c"IOService".as_ptr(), &mut parent) } == 0)
            .then(|| Io::owned(parent))
            .flatten()
    }
}

fn matching_services(matching: CFTypeRef) -> Vec<Io> {
    let mut iterator = 0;
    // SAFETY: the call consumes the matching dictionary
    if matching.is_null()
        || unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) } != 0
    {
        return Vec::new();
    }
    let iterator = Io(iterator);
    std::iter::from_fn(|| Io::owned(unsafe { IOIteratorNext(iterator.0) })).collect()
}

// ---- dlopen --------------------------------------------------------------------------------

/// Opens a system framework once; never closed (ObjC classes must stay loaded)
fn framework(path: &'static CStr) -> Option<usize> {
    // SAFETY: a valid path; the handle is only used with dlsym
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY) };
    (!handle.is_null()).then_some(handle as usize)
}

/// Looks up a function. T must be the function's exact pointer type
unsafe fn symbol<T: Copy>(handle: usize, name: &CStr) -> Option<T> {
    // SAFETY: the caller names the right type for the symbol
    unsafe {
        let pointer = libc::dlsym(handle as *mut c_void, name.as_ptr());
        (!pointer.is_null()).then(|| std::mem::transmute_copy(&pointer))
    }
}

// ---- sysctl --------------------------------------------------------------------------------

fn sysctl_raw(name: &str) -> Option<Vec<u8>> {
    let name = CString::new(name).ok()?;
    let mut size = 0;
    // SAFETY: size query, then a read into a buffer of that size
    unsafe {
        if libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buffer.truncate(size);
        Some(buffer)
    }
}

fn sysctl_string(name: &str) -> Option<String> {
    let raw = sysctl_raw(name)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = String::from_utf8_lossy(&raw[..end]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let raw = sysctl_raw(name)?;
    match raw.len() {
        4 => Some(u32::from_ne_bytes(raw.try_into().ok()?) as u64),
        8 => Some(u64::from_ne_bytes(raw.try_into().ok()?)),
        _ => None,
    }
}

fn sysctl_struct<T>(name: &str) -> Option<T> {
    let raw = sysctl_raw(name)?;
    // SAFETY: sized exactly like T, read unaligned
    (raw.len() == size_of::<T>())
        .then(|| unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const T) })
}

// ---- OS, host, uptime ----------------------------------------------------------------------

pub fn codename(version: &str) -> Option<&'static str> {
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next().unwrap_or("0");
    Some(match (major, minor) {
        ("27", _) => "Golden Gate",
        ("26", _) => "Tahoe",
        ("15", _) => "Sequoia",
        ("14", _) => "Sonoma",
        ("13", _) => "Ventura",
        ("12", _) => "Monterey",
        // Big Sur reports 10.16 to programs built with old SDKs
        ("11", _) | ("10", "16") => "Big Sur",
        ("10", "15") => "Catalina",
        ("10", "14") => "Mojave",
        ("10", "13") => "High Sierra",
        _ => return None,
    })
}

pub fn os() -> Os {
    let version = sysctl_string("kern.osproductversion");
    let codename = version.as_deref().and_then(codename).map(str::to_string);
    let pretty = match (&codename, &version) {
        (Some(c), Some(v)) => format!("macOS {c} {v}"),
        (None, Some(v)) => format!("macOS {v}"),
        _ => String::from("macOS"),
    };
    Os {
        id: "macos".into(),
        like: Vec::new(),
        pretty,
        version,
        codename,
        build: sysctl_string("kern.osversion"),
    }
}

/// "MacBook Pro (16-inch, M5 Pro)" from the device tree on Apple Silicon, "MacBookPro16,1" on Intel
pub fn host_model() -> Option<String> {
    Io::from_path(c"IODeviceTree:/product")
        .and_then(|product| product.property("product-name"))
        .and_then(|name| cf_text(name.0))
        .or_else(|| sysctl_string("hw.model"))
}

pub fn uptime_seconds() -> Option<u64> {
    let boot: libc::timeval = sysctl_struct("kern.boottime")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    now.checked_sub(boot.tv_sec as u64)
}

// ---- CPU, GPU ------------------------------------------------------------------------------

pub fn cpu() -> Option<Cpu> {
    let name = crate::format::clean_cpu_name(&sysctl_string("machdep.cpu.brand_string")?);
    // Apple Silicon has performance levels, fastest first: [6, 12] on an M5 Pro
    let levels = sysctl_u64("hw.nperflevels").unwrap_or(1);
    let cores = if levels >= 2 {
        (0..levels)
            .map(|l| sysctl_u64(&format!("hw.perflevel{l}.physicalcpu")).unwrap_or(0) as u32)
            .collect()
    } else {
        sysctl_u64("hw.physicalcpu")
            .map(|c| vec![c as u32])
            .unwrap_or_default()
    };
    // Intel publishes its top speed; Apple Silicon keeps the P-cluster's in the power manager
    let max_mhz = sysctl_u64("hw.cpufrequency_max")
        .map(|hz| hz / 1_000_000)
        .or_else(|| pmgr_max_mhz("voltage-states5-sram"))
        .unwrap_or(0);
    Some(Cpu {
        name,
        cores,
        max_mhz,
    })
}

/// Apple Silicon's power manager lists each cluster's (frequency, voltage) states as pairs of
/// little-endian u32: voltage-states5-sram for the P-cores, 9 for the GPU. Frequencies are Hz on
/// recent chips and kHz on older ones
fn pmgr_max_mhz(key: &str) -> Option<u64> {
    static PMGR: OnceLock<Option<Io>> = OnceLock::new();
    // the device tree path is ~15 µs, name matching ~65 µs
    let pmgr = PMGR
        .get_or_init(|| {
            Io::from_path(c"IODeviceTree:/arm-io/pmgr")
                .or_else(|| {
                    Io::owned(unsafe {
                        IOServiceGetMatchingService(0, IOServiceNameMatching(c"pmgr".as_ptr()))
                    })
                })
                .filter(|p| p.conforms(c"AppleARMIODevice"))
        })
        .as_ref()?;
    let data = cf_bytes(pmgr.property(key)?.0);
    max_state_mhz(&data)
}

pub fn max_state_mhz(data: &[u8]) -> Option<u64> {
    if data.is_empty() || !data.len().is_multiple_of(8) {
        return None;
    }
    let top = data
        .as_chunks::<8>()
        .0
        .iter()
        .filter_map(|pair| pair[..4].try_into().ok().map(u32::from_le_bytes))
        .max()? as u64;
    let mhz = if top > 100_000_000 {
        top / 1_000_000
    } else {
        top / 1000
    };
    (mhz > 0).then_some(mhz)
}

// OnceLock in a static needs Sync; registry handles are plain integers safe to share
unsafe impl Sync for Io {}
unsafe impl Send for Io {}

pub fn gpus() -> Vec<Gpu> {
    // SAFETY: the matching dictionary is mutable and consumed by matching_services
    let matching = unsafe {
        let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
        if let (false, Some(key), Some(value)) = (
            matching.is_null(),
            Cf::string("IOMatchCategory"),
            Cf::string("IOAccelerator"),
        ) {
            CFDictionarySetValue(matching, key.0, value.0);
        }
        matching
    };
    matching_services(matching)
        .into_iter()
        .filter_map(|accelerator| {
            let cores = accelerator
                .property("gpu-core-count")
                .and_then(|c| cf_i64(c.0))
                .filter(|&c| c > 0)
                .map(|c| c as u32);
            // Apple Silicon names itself; Intel and AMD GPUs are named by their PCI parent
            let (name, vendor) = match accelerator.property("model").and_then(|m| cf_text(m.0)) {
                Some(name) => (
                    name,
                    accelerator.property("vendor-id").and_then(|v| cf_i64(v.0)),
                ),
                None => {
                    let mut parent = accelerator.parent();
                    while let Some(entry) = parent.take() {
                        if entry.conforms(c"IOPCIDevice") {
                            parent = Some(entry);
                            break;
                        }
                        parent = entry.parent();
                    }
                    let pci = parent?;
                    (
                        cf_text(pci.property("model")?.0)?,
                        pci.property("vendor-id").and_then(|v| cf_i64(v.0)),
                    )
                },
            };
            let max_mhz = if vendor == Some(0x106b) || name.starts_with("Apple") {
                pmgr_max_mhz("voltage-states9-sram").unwrap_or(0)
            } else {
                0
            };
            Some(Gpu {
                name,
                cores,
                max_mhz,
            })
        })
        .collect()
}

// ---- displays ------------------------------------------------------------------------------

type CgId = u32;

struct Cg {
    online: unsafe extern "C" fn(u32, *mut CgId, *mut u32) -> i32,
    copy_mode: unsafe extern "C" fn(CgId) -> CFTypeRef,
    width: unsafe extern "C" fn(CFTypeRef) -> usize,
    height: unsafe extern "C" fn(CFTypeRef) -> usize,
    refresh: unsafe extern "C" fn(CFTypeRef) -> f64,
    release_mode: unsafe extern "C" fn(CFTypeRef),
    is_main: unsafe extern "C" fn(CgId) -> u32,
    is_builtin: unsafe extern "C" fn(CgId) -> u32,
    model: unsafe extern "C" fn(CgId) -> u32,
}

fn core_graphics() -> Option<Cg> {
    let cg = framework(c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")?;
    // SAFETY: the types match CoreGraphics' declarations
    unsafe {
        Some(Cg {
            online: symbol(cg, c"CGGetOnlineDisplayList")?,
            copy_mode: symbol(cg, c"CGDisplayCopyDisplayMode")?,
            width: symbol(cg, c"CGDisplayModeGetPixelWidth")?,
            height: symbol(cg, c"CGDisplayModeGetPixelHeight")?,
            refresh: symbol(cg, c"CGDisplayModeGetRefreshRate")?,
            release_mode: symbol(cg, c"CGDisplayModeRelease")?,
            is_main: symbol(cg, c"CGDisplayIsMain")?,
            is_builtin: symbol(cg, c"CGDisplayIsBuiltin")?,
            model: symbol(cg, c"CGDisplayModelNumber")?,
        })
    }
}

/// Every display. Apple Silicon answers from IOKit's framebuffers in ~0.2 ms; CoreGraphics'
/// first call connects to the window server (~10 ms), so it is only the fallback (Intel Macs)
pub fn screens() -> Vec<Screen> {
    let screens = framebuffer_screens();
    if screens.is_empty() {
        core_graphics_screens()
    } else {
        screens
    }
}

/// The DCP framebuffers that drive a display: their output size is the panel's own resolution
/// (3456x2234 on a 16" MacBook Pro, whatever scaling is picked) and their shortest refresh
/// interval, a 32.32 fixed-point fraction of a second, gives the top refresh rate (120 Hz for
/// ProMotion, which CoreGraphics also reports)
fn framebuffer_screens() -> Vec<Screen> {
    let mut screens: Vec<Screen> =
        matching_services(unsafe { IOServiceMatching(c"IOMobileFramebuffer".as_ptr()) })
            .into_iter()
            .filter_map(|fb| {
                let width = fb
                    .property("DisplayWidth")
                    .and_then(|v| cf_i64(v.0))
                    .filter(|&w| w > 0)?;
                let height = fb
                    .property("DisplayHeight")
                    .and_then(|v| cf_i64(v.0))
                    .filter(|&h| h > 0)?;
                let external = fb
                    .property("external")
                    .and_then(|v| cf_bool(v.0))
                    .unwrap_or(false);
                let refresh_hz = fb
                    .property("IOMFBDisplayRefresh")
                    .and_then(|r| cf_i64(dict_get(r.0, "displayMinRefreshInterval")))
                    .filter(|&i| i > 0)
                    .map(|interval| ((1u64 << 32) as f64 / interval as f64).round() as u32);
                let name = fb
                    .property("DisplayAttributes")
                    .and_then(|a| {
                        cf_text(dict_get(dict_get(a.0, "ProductAttributes"), "ProductName"))
                    })
                    .unwrap_or_else(|| {
                        String::from(if external { "External" } else { "Built-in" })
                    });
                Some(Screen {
                    name,
                    width: width as u32,
                    height: height as u32,
                    refresh_hz,
                    primary: !external,
                })
            })
            .collect();
    // the built-in panel first, then externals in registry order
    screens.sort_by_key(|s| !s.primary);
    screens
}

/// Every display, sleeping ones too (the online list, not the active one, so this works over SSH
/// while the screens are off). Sizes are backing pixels: 3456x2234 for "looks like 1728x1117"
fn core_graphics_screens() -> Vec<Screen> {
    let Some(cg) = core_graphics() else {
        return Vec::new();
    };
    let mut ids = [0 as CgId; 16];
    let mut count = 0;
    // SAFETY: the buffer holds 16 ids
    if unsafe { (cg.online)(16, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    let ids = &ids[..(count as usize).min(16)];
    // Names cost ~1.5 ms (CoreDisplay's first call) and only matter with several displays
    let names = ids.len() > 1;
    ids.iter()
        .filter_map(|&id| {
            // SAFETY: the mode is a +1 reference released below
            let (width, height, refresh) = unsafe {
                let mode = (cg.copy_mode)(id);
                if mode.is_null() {
                    return None;
                }
                let values = ((cg.width)(mode), (cg.height)(mode), (cg.refresh)(mode));
                (cg.release_mode)(mode);
                values
            };
            let refresh_hz = (refresh > 0.0).then(|| refresh.round() as u32);
            // SAFETY: plain queries on a display id
            let (primary, builtin, model) = unsafe {
                (
                    (cg.is_main)(id) != 0,
                    (cg.is_builtin)(id) != 0,
                    (cg.model)(id),
                )
            };
            let mut name = if names {
                display_name(id).unwrap_or_default()
            } else {
                String::new()
            };
            if name == "Color LCD" && builtin && is_liquid_retina_xdr(model, refresh_hz) {
                name = String::from("Liquid Retina XDR");
            }
            Some(Screen {
                name,
                width: width as u32,
                height: height as u32,
                refresh_hz,
                primary,
            })
        })
        .collect()
}

/// The product name from CoreDisplay's info dictionary. Private, so looked up at runtime: if
/// Apple removes it the line just loses its names
fn display_name(id: CgId) -> Option<String> {
    let core_display = framework(c"/System/Library/Frameworks/CoreDisplay.framework/CoreDisplay")?;
    // SAFETY: returns a +1 dictionary
    let info: unsafe extern "C" fn(CgId) -> CFTypeRef =
        unsafe { symbol(core_display, c"CoreDisplay_DisplayCreateInfoDictionary")? };
    let info = Cf::owned(unsafe { info(id) })?;
    let names = dict_get(info.0, "DisplayProductName");
    if let Some(name) = cf_text(dict_get(names, "en_US")) {
        return Some(name);
    }
    if !type_is(names, unsafe { CFDictionaryGetTypeID() }) {
        return None;
    }
    // SAFETY: names is a dictionary; get-rule values
    unsafe {
        let count = CFDictionaryGetCount(names).max(0) as usize;
        let mut values = vec![std::ptr::null(); count];
        CFDictionaryGetKeysAndValues(names, std::ptr::null_mut(), values.as_mut_ptr());
        values.into_iter().find_map(cf_text)
    }
}

/// MacBook Pro panels: XDR (mini-LED) models, plain Liquid Retina (MacBook Air) never. An unknown
/// model counts as XDR at 120 Hz, which only the XDR panels run
pub fn is_liquid_retina_xdr(model: u32, refresh_hz: Option<u32>) -> bool {
    match model {
        0xa04e..=0xa051 | 0xa057..=0xa05f | 0xa061 => true,
        0xa052..=0xa055 | 0xa062..=0xa067 => false,
        _ => refresh_hz.is_some_and(|hz| hz >= 120),
    }
}

// ---- memory --------------------------------------------------------------------------------

/// Used the way Activity Monitor counts it: app memory + wired + compressed
pub fn memory() -> Option<Usage> {
    let total = sysctl_u64("hw.memsize")?;
    let page = sysctl_u64("hw.pagesize")?;
    let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: host_statistics64 fills count words of the struct
    let ok = unsafe {
        #[allow(deprecated)]
        let host = libc::mach_host_self();
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            stats.as_mut_ptr() as *mut i32,
            &mut count,
        )
    } == 0;
    if !ok {
        return None;
    }
    // SAFETY: filled above
    let s = unsafe { stats.assume_init() };
    let pages = (s.internal_page_count as u64).saturating_sub(s.purgeable_count as u64)
        + s.wire_count as u64
        + s.compressor_page_count as u64;
    Some(Usage {
        used: (pages * page).min(total),
        total,
    })
}

pub fn swap() -> Option<Usage> {
    let usage: libc::xsw_usage = sysctl_struct("vm.swapusage")?;
    Some(Usage {
        used: usage.xsu_used,
        total: usage.xsu_total,
    })
}

pub fn disk_path(path: &str) -> String {
    path.to_string()
}

// ---- battery -------------------------------------------------------------------------------

pub fn battery(with_adapter: bool) -> Option<Battery> {
    // Desktops don't publish the service. Keys are read one by one: copying the whole property
    // table (with its big BatteryData) costs ~7x more
    // SAFETY: the matching dictionary is consumed
    let service = Io::owned(unsafe {
        IOServiceGetMatchingService(0, IOServiceMatching(c"AppleSmartBattery".as_ptr()))
    })?;
    let flag = |key: &str| service.property(key).and_then(|v| cf_bool(v.0));
    let number = |key: &str| service.property(key).and_then(|v| cf_i64(v.0));
    // a MacBook with its battery removed still publishes the service
    if flag("BatteryInstalled") == Some(false) {
        return None;
    }
    let percent = battery_percent(number("CurrentCapacity")?, number("MaxCapacity")?)?;
    let external = flag("ExternalConnected").unwrap_or(false);
    let charge = if flag("IsCharging").unwrap_or(false) {
        Charge::Charging
    } else if !external {
        Charge::Discharging
    } else if flag("FullyCharged").unwrap_or(false) {
        Charge::Full
    } else {
        // held below full by Optimized Charging or a charge limit
        Charge::NotCharging
    };
    // AdapterDetails can outlive an unplug
    let adapter = (with_adapter && external)
        .then(|| {
            let details = service.property("AdapterDetails")?;
            let watts = cf_i64(dict_get(details.0, "Watts"))?;
            adapter_label(watts, cf_text(dict_get(details.0, "Name")).as_deref())
        })
        .flatten();
    Some(Battery {
        percent,
        charge,
        adapter,
    })
}

/// Apple Silicon reports percent (max 100), Intel mAh: both round the same way
pub fn battery_percent(current: i64, max: i64) -> Option<u8> {
    if max <= 0 || current < 0 {
        return None;
    }
    let percent = current.checked_mul(100)?.checked_add(max / 2)? / max;
    Some(percent.min(100) as u8)
}

/// "140W USB-C Power Adapter" as named, "96W" unnamed, "30W (Name)" when the name doesn't start
/// with its wattage. Control characters are dropped so a name can't carry escape codes
pub fn adapter_label(watts: i64, name: Option<&str>) -> Option<String> {
    if watts <= 0 {
        return None;
    }
    let name: String = name
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = name.trim();
    let prefix = format!("{watts}W");
    Some(if name.is_empty() {
        prefix
    } else if name.starts_with(&prefix) {
        name.to_string()
    } else {
        format!("{prefix} ({name})")
    })
}

pub fn power_draw_watts() -> Option<f64> {
    None
}

// ---- packages ------------------------------------------------------------------------------

fn count_dirs(path: &str) -> usize {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}

pub fn packages() -> Vec<Packages> {
    let prefix = if std::path::Path::new("/opt/homebrew/Cellar").exists() {
        "/opt/homebrew"
    } else {
        "/usr/local"
    };
    vec![
        Packages {
            manager: "brew",
            count: count_dirs(&format!("{prefix}/Cellar")),
        },
        Packages {
            manager: "brew-cask",
            count: count_dirs(&format!("{prefix}/Caskroom")),
        },
        Packages {
            manager: "port",
            count: count_dirs("/opt/local/var/macports/software"),
        },
    ]
}

// ---- processes -----------------------------------------------------------------------------

/// (parent pid, executable name). The short BSD info works on other users' processes (the full
/// one is EPERM), so the walk doesn't stop at sudo or login
pub fn process_parent(pid: u32) -> Option<(u32, String)> {
    let mut info = MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    let size = size_of::<libc::proc_bsdshortinfo>() as i32;
    // SAFETY: proc_pidinfo writes at most size bytes
    let written = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr() as *mut c_void,
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: fully written
    let info = unsafe { info.assume_init() };
    // SAFETY: pbsi_comm is NUL-padded, and the slice stays within the array
    let comm = unsafe {
        std::slice::from_raw_parts(info.pbsi_comm.as_ptr() as *const u8, info.pbsi_comm.len())
    };
    let end = comm.iter().position(|&b| b == 0).unwrap_or(comm.len());
    Some((
        info.pbsi_ppid,
        String::from_utf8_lossy(&comm[..end]).into_owned(),
    ))
}

/// The executable, symlinks resolved (Homebrew's Cellar path carries the version)
pub fn process_path(pid: u32) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is PROC_PIDPATHINFO_MAXSIZE bytes
    let length = unsafe {
        libc::proc_pidpath(
            pid as i32,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as u32,
        )
    };
    (length > 0).then(|| String::from_utf8_lossy(&buffer[..length as usize]).into_owned())
}

/// argv[0]. macOS' /bin/sh re-executes /bin/bash, only argv[0] still says "sh"
pub fn process_argv0(pid: u32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut size = 0;
    // SAFETY: size query, then a read into a buffer of that size
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buffer.truncate(size);
        parse_procargs_argv0(&buffer)
    }
}

/// KERN_PROCARGS2 is argc (4 bytes), the exec path, NUL padding, then argv
pub fn parse_procargs_argv0(data: &[u8]) -> Option<String> {
    let rest = data.get(4..)?;
    let path_end = rest.iter().position(|&b| b == 0)?;
    let rest = &rest[path_end..];
    let start = rest.iter().position(|&b| b != 0)?;
    let rest = &rest[start..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    (end > 0).then(|| String::from_utf8_lossy(&rest[..end]).into_owned())
}

// ---- routes --------------------------------------------------------------------------------

/// Reads the routing table entries with `flags` set (NET_RT_FLAGS), so a VPN that pushes
/// thousands of routes costs nothing extra. Retries when the table grows between the calls
fn route_dump(family: i32, flags: i32) -> Option<Vec<u8>> {
    let mut mib = [
        libc::CTL_NET,
        libc::PF_ROUTE,
        0,
        family,
        libc::NET_RT_FLAGS,
        flags,
    ];
    for _ in 0..3 {
        let mut size = 0;
        // SAFETY: size query, then a read with headroom
        unsafe {
            if libc::sysctl(
                mib.as_mut_ptr(),
                6,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            size += 1024;
            let mut buffer = vec![0u8; size];
            if libc::sysctl(
                mib.as_mut_ptr(),
                6,
                buffer.as_mut_ptr() as *mut c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            ) == 0
            {
                buffer.truncate(size);
                return Some(buffer);
            }
            if *libc::__error() != libc::ENOMEM {
                return None;
            }
        }
    }
    None
}

/// The main default route's interface. macOS also keeps an interface-scoped default for every
/// interface with a router (RTF_IFSCOPE); the unscoped one is where traffic goes
pub fn default_route(v6: bool) -> Option<String> {
    static ROUTES: OnceLock<[Option<String>; 2]> = OnceLock::new();
    ROUTES.get_or_init(|| [find_default_route(false), find_default_route(true)])[v6 as usize]
        .clone()
}

fn find_default_route(v6: bool) -> Option<String> {
    let family = if v6 { libc::AF_INET6 } else { libc::AF_INET };
    let dump = route_dump(family, libc::RTF_GATEWAY)?;
    let header = size_of::<libc::rt_msghdr>();
    let mut at = 0;
    while at + header <= dump.len() {
        // SAFETY: bounds checked, read unaligned
        let message: libc::rt_msghdr =
            unsafe { std::ptr::read_unaligned(dump.as_ptr().add(at) as *const _) };
        let length = message.rtm_msglen as usize;
        if length < header || at + length > dump.len() {
            break;
        }
        let body = &dump[at + header..at + length];
        at += length;
        let wanted = libc::RTF_UP | libc::RTF_GATEWAY;
        if message.rtm_flags & wanted != wanted || message.rtm_flags & libc::RTF_IFSCOPE != 0 {
            continue;
        }
        if is_default(body, message.rtm_addrs, v6) {
            let mut name = [0 as c_char; libc::IF_NAMESIZE];
            // SAFETY: the buffer is IF_NAMESIZE bytes
            if !unsafe { libc::if_indextoname(message.rtm_index as u32, name.as_mut_ptr()) }
                .is_null()
            {
                return Some(
                    unsafe { CStr::from_ptr(name.as_ptr()) }
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    None
}

/// The sockaddrs after a route message, one per set bit of `addrs`, each padded to 4 bytes (an
/// empty one still takes 4)
fn route_sockaddrs(body: &[u8], addrs: i32) -> Vec<(i32, &[u8])> {
    let mut found = Vec::new();
    let mut at = 0;
    for bit in 0..libc::RTAX_MAX {
        if addrs & (1 << bit) == 0 {
            continue;
        }
        let Some(&length) = body.get(at) else { break };
        let length = length as usize;
        let padded = if length == 0 { 4 } else { (length + 3) & !3 };
        found.push((bit, body.get(at..at + length).unwrap_or(&[])));
        at += padded;
    }
    found
}

/// Destination all zeros and an all-zero (or empty) netmask. Rejects a VPN's 0.0.0.0/1 halves
fn is_default(body: &[u8], addrs: i32, v6: bool) -> bool {
    let (offset, size) = if v6 { (8, 16) } else { (4, 4) };
    let addresses = route_sockaddrs(body, addrs);
    let zero = |bytes: &[u8]| {
        bytes
            .get(offset..(offset + size).min(bytes.len()))
            .is_none_or(|b| b.iter().all(|&x| x == 0))
    };
    let destination = addresses.iter().find(|(bit, _)| *bit == libc::RTAX_DST);
    let netmask = addresses.iter().find(|(bit, _)| *bit == libc::RTAX_NETMASK);
    destination.is_some_and(|(_, d)| d.len() >= offset && zero(d))
        && netmask.is_none_or(|(_, m)| zero(m))
}

const SIOCGIFAFLAG_IN6: libc::c_ulong = 0xc120_6949;

#[repr(C)]
struct In6Ifreq {
    name: [c_char; libc::IFNAMSIZ],
    data: [u8; 272],
}

/// The kernel's flags for one IPv6 address
pub fn ipv6_state(interface: &str, address: &Ipv6Addr) -> V6State {
    let Some(flags) = ipv6_flags(interface, address) else {
        return V6State::Stable;
    };
    // tentative, duplicated, detached, deprecated
    if flags & (0x2 | 0x4 | 0x8 | 0x10) != 0 {
        V6State::Unusable
    } else if flags & 0x80 != 0 {
        V6State::Temporary
    } else {
        V6State::Stable
    }
}

fn ipv6_flags(interface: &str, address: &Ipv6Addr) -> Option<i32> {
    if interface.len() >= libc::IFNAMSIZ {
        return None;
    }
    let mut request = In6Ifreq {
        name: [0; libc::IFNAMSIZ],
        data: [0; 272],
    };
    for (slot, byte) in request.name.iter_mut().zip(interface.bytes()) {
        *slot = byte as c_char;
    }
    let mut sin6: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    sin6.sin6_len = size_of::<libc::sockaddr_in6>() as u8;
    sin6.sin6_family = libc::AF_INET6 as u8;
    sin6.sin6_addr.s6_addr = address.octets();
    // SAFETY: sockaddr_in6 (28 bytes) fits the 272-byte union
    unsafe {
        std::ptr::copy_nonoverlapping(
            &sin6 as *const _ as *const u8,
            request.data.as_mut_ptr(),
            size_of::<libc::sockaddr_in6>(),
        );
        let socket = libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0);
        if socket < 0 {
            return None;
        }
        let ok = libc::ioctl(socket, SIOCGIFAFLAG_IN6, &mut request) == 0;
        libc::close(socket);
        ok.then(|| {
            i32::from_ne_bytes([
                request.data[0],
                request.data[1],
                request.data[2],
                request.data[3],
            ])
        })
    }
}

// ---- Wi-Fi ---------------------------------------------------------------------------------

const SIOCGIFMEDIA: libc::c_ulong = 0xc02c_6938;

#[repr(C, packed(4))]
struct IfMediaRequest {
    name: [c_char; libc::IFNAMSIZ],
    current: i32,
    mask: i32,
    status: i32,
    active: i32,
    count: i32,
    list: *mut i32,
}

/// One ioctl says whether an interface is Wi-Fi and joined to a network (CoreWLAN takes ms)
fn is_associated_wifi(interface: &str) -> bool {
    if interface.len() >= libc::IFNAMSIZ {
        return false;
    }
    let mut request = IfMediaRequest {
        name: [0; libc::IFNAMSIZ],
        current: 0,
        mask: 0,
        status: 0,
        active: 0,
        count: 0,
        list: std::ptr::null_mut(),
    };
    for (slot, byte) in request.name.iter_mut().zip(interface.bytes()) {
        *slot = byte as c_char;
    }
    // SAFETY: a plain query into the request struct
    let ok = unsafe {
        let socket = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if socket < 0 {
            return false;
        }
        let ok = libc::ioctl(socket, SIOCGIFMEDIA, &mut request) == 0;
        libc::close(socket);
        ok
    };
    let (active, status) = (request.active, request.status);
    // IEEE 802.11 media, valid and active
    ok && active & 0xe0 == 0x80 && status & 3 == 3
}

/// Network names macOS blanks out without Location Services. Control characters are dropped
pub fn sanitize_ssid(name: &str) -> Option<String> {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    (!clean.is_empty() && clean != "<redacted>").then(|| clean.to_string())
}

pub fn wifi_ssid() -> Option<String> {
    let mut seen = Vec::new();
    for a in net::interfaces() {
        if a.address.is_ipv4() && !net::is_virtual(&a.interface) && !seen.contains(&a.interface) {
            seen.push(a.interface.clone());
        }
    }
    let interface = seen.into_iter().find(|i| is_associated_wifi(i))?;
    scan_record_ssid(&interface).or_else(|| corewlan_ssid(&interface))
}

/// configd's last scan record for the interface, an NSKeyedArchiver archive holding SSID_STR.
/// ~1 ms, and it works without Location Services, unlike CoreWLAN's ssid
fn scan_record_ssid(interface: &str) -> Option<String> {
    let sc =
        framework(c"/System/Library/Frameworks/SystemConfiguration.framework/SystemConfiguration")?;
    // SAFETY: matches SCDynamicStoreCopyValue's declaration; copy rule
    let copy_value: unsafe extern "C" fn(CFTypeRef, CFTypeRef) -> CFTypeRef =
        unsafe { symbol(sc, c"SCDynamicStoreCopyValue")? };
    let key = Cf::string(&format!("State:/Network/Interface/{interface}/AirPort"))?;
    let state = Cf::owned(unsafe { copy_value(std::ptr::null(), key.0) })?;
    let record = dict_get(state.0, "CachedScanRecord");
    if !type_is(record, unsafe { CFDataGetTypeID() }) {
        return None;
    }
    let objc = Objc::load()?;
    objc.autoreleased(|| {
        let classes: Vec<*const c_void> =
            ["NSDictionary", "NSArray", "NSString", "NSData", "NSNumber"]
                .iter()
                .filter_map(|c| objc.class(c))
                .collect();
        let set = objc.class("NSSet")?;
        // +[NSSet setWithObjects:count:]
        let set: *const c_void = unsafe {
            objc.send2(
                set,
                "setWithObjects:count:",
                classes.as_ptr() as *const c_void,
                classes.len() as *const c_void,
            )
        };
        let unarchiver = objc.class("NSKeyedUnarchiver")?;
        // +[NSKeyedUnarchiver unarchivedObjectOfClasses:fromData:error:], secure decoding
        let object: *const c_void = unsafe {
            objc.send3(
                unarchiver,
                "unarchivedObjectOfClasses:fromData:error:",
                set,
                record,
                std::ptr::null(),
            )
        };
        // NSDictionary and NSString are toll-free bridged with their CF types
        cf_text(dict_get(object, "SSID_STR")).and_then(|s| sanitize_ssid(&s))
    })
}

/// CoreWLAN (~18 ms, so only when the scan record had nothing). Its ssid needs Location Services;
/// without it, the most recently joined known network is the best guess, as fastfetch does
fn corewlan_ssid(interface: &str) -> Option<String> {
    let objc = Objc::load()?;
    framework(c"/System/Library/Frameworks/CoreWLAN.framework/CoreWLAN")?;
    objc.autoreleased(|| unsafe {
        let client: *const c_void = objc.send0(objc.class("CWWiFiClient")?, "sharedWiFiClient");
        let name = Cf::string(interface)?;
        let wifi: *const c_void = objc.send1(client, "interfaceWithName:", name.0);
        if wifi.is_null() {
            return None;
        }
        if let Some(ssid) = cf_text(objc.send0(wifi, "ssid")).and_then(|s| sanitize_ssid(&s)) {
            return Some(ssid);
        }
        // kCWInterfaceModeStation
        if objc.send0(wifi, "interfaceMode") as usize != 1 {
            return None;
        }
        let configuration: *const c_void = objc.send0(wifi, "configuration");
        let profiles: *const c_void = objc.send0(configuration, "networkProfiles");
        let first: *const c_void = objc.send0(profiles, "firstObject");
        cf_text(objc.send0(first, "ssid")).and_then(|s| sanitize_ssid(&s))
    })
}

/// Just enough of the Objective-C runtime to call a few methods, loaded with Foundation
struct Objc {
    get_class: unsafe extern "C" fn(*const c_char) -> *const c_void,
    selector: unsafe extern "C" fn(*const c_char) -> *const c_void,
    send: *const c_void,
    push: unsafe extern "C" fn() -> *mut c_void,
    pop: unsafe extern "C" fn(*mut c_void),
}

impl Objc {
    fn load() -> Option<Objc> {
        let foundation = framework(c"/System/Library/Frameworks/Foundation.framework/Foundation")?;
        let runtime = framework(c"/usr/lib/libobjc.A.dylib").unwrap_or(foundation);
        // SAFETY: the runtime's documented signatures
        unsafe {
            Some(Objc {
                get_class: symbol(runtime, c"objc_getClass")?,
                selector: symbol(runtime, c"sel_registerName")?,
                send: symbol(runtime, c"objc_msgSend")?,
                push: symbol(runtime, c"objc_autoreleasePoolPush")?,
                pop: symbol(runtime, c"objc_autoreleasePoolPop")?,
            })
        }
    }

    fn class(&self, name: &str) -> Option<*const c_void> {
        let name = CString::new(name).ok()?;
        // SAFETY: a valid class name
        let class = unsafe { (self.get_class)(name.as_ptr()) };
        (!class.is_null()).then_some(class)
    }

    fn sel(&self, name: &str) -> *const c_void {
        let name = CString::new(name).unwrap_or_default();
        // SAFETY: a valid selector name
        unsafe { (self.selector)(name.as_ptr()) }
    }

    /// Messages to nil return nil, so a chain stops harmlessly at the first missing object
    unsafe fn send0(&self, receiver: *const c_void, selector: &str) -> *const c_void {
        if receiver.is_null() {
            return std::ptr::null();
        }
        // SAFETY: every method called through here takes no arguments and returns an object or
        // an integer
        unsafe {
            let f: unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void =
                std::mem::transmute(self.send);
            f(receiver, self.sel(selector))
        }
    }

    unsafe fn send1(
        &self,
        receiver: *const c_void,
        selector: &str,
        a: *const c_void,
    ) -> *const c_void {
        if receiver.is_null() {
            return std::ptr::null();
        }
        // SAFETY: one pointer-sized argument
        unsafe {
            let f: unsafe extern "C" fn(
                *const c_void,
                *const c_void,
                *const c_void,
            ) -> *const c_void = std::mem::transmute(self.send);
            f(receiver, self.sel(selector), a)
        }
    }

    unsafe fn send2(
        &self,
        receiver: *const c_void,
        selector: &str,
        a: *const c_void,
        b: *const c_void,
    ) -> *const c_void {
        // SAFETY: two pointer-sized arguments (an NSUInteger count is pointer-sized)
        unsafe {
            let f: unsafe extern "C" fn(
                *const c_void,
                *const c_void,
                *const c_void,
                *const c_void,
            ) -> *const c_void = std::mem::transmute(self.send);
            f(receiver, self.sel(selector), a, b)
        }
    }

    unsafe fn send3(
        &self,
        receiver: *const c_void,
        selector: &str,
        a: *const c_void,
        b: *const c_void,
        c: *const c_void,
    ) -> *const c_void {
        // SAFETY: three pointer-sized arguments
        unsafe {
            let f: unsafe extern "C" fn(
                *const c_void,
                *const c_void,
                *const c_void,
                *const c_void,
                *const c_void,
            ) -> *const c_void = std::mem::transmute(self.send);
            f(receiver, self.sel(selector), a, b, c)
        }
    }

    fn autoreleased<T>(&self, body: impl FnOnce() -> Option<T>) -> Option<T> {
        // SAFETY: push and pop are balanced
        let pool = unsafe { (self.push)() };
        let result = body();
        unsafe { (self.pop)(pool) };
        result
    }
}
