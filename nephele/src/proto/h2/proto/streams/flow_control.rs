use std::fmt;

use crate::proto::h2::frame::Reason;
use crate::proto::h2::proto::{WindowSize, MAX_WINDOW_SIZE};

const UNCLAIMED_NUMERATOR: i32 = 1;
const UNCLAIMED_DENOMINATOR: i32 = 2;

#[derive(Copy, Clone, Debug)]
pub struct FlowControl {
    window_size: Window,
    available: Window,
}

impl FlowControl {
    pub fn new() -> FlowControl {
        FlowControl {
            window_size: Window(0),
            available: Window(0),
        }
    }

    pub fn window_size(&self) -> WindowSize {
        self.window_size.as_size()
    }

    pub fn available(&self) -> Window {
        self.available
    }

    pub fn has_unavailable(&self) -> bool {
        if self.window_size < 0 {
            return false;
        }

        self.window_size > self.available
    }

    pub fn claim_capacity(&mut self, capacity: WindowSize) {
        self.available -= capacity;
    }

    pub fn assign_capacity(&mut self, capacity: WindowSize) {
        self.available += capacity;
    }

    pub fn unclaimed_capacity(&self) -> Option<WindowSize> {
        let available = self.available;

        if self.window_size >= available {
            return None;
        }

        let unclaimed = available.0 - self.window_size.0;
        let threshold = self.window_size.0 / UNCLAIMED_DENOMINATOR * UNCLAIMED_NUMERATOR;

        if unclaimed < threshold {
            None
        } else {
            Some(unclaimed as WindowSize)
        }
    }

    pub fn inc_window(&mut self, sz: WindowSize) -> Result<(), Reason> {
        let (val, overflow) = self.window_size.0.overflowing_add(sz as i32);

        if overflow {
            return Err(Reason::FLOW_CONTROL_ERROR);
        }

        if val > MAX_WINDOW_SIZE as i32 {
            return Err(Reason::FLOW_CONTROL_ERROR);
        }

        tracing::trace!(
            "inc_window; sz={}; old={}; new={}",
            sz,
            self.window_size,
            val
        );

        self.window_size = Window(val);
        Ok(())
    }

    pub fn dec_send_window(&mut self, sz: WindowSize) {
        tracing::trace!(
            "dec_window; sz={}; window={}, available={}",
            sz,
            self.window_size,
            self.available
        );
        self.window_size -= sz;
    }

    pub fn dec_recv_window(&mut self, sz: WindowSize) {
        tracing::trace!(
            "dec_recv_window; sz={}; window={}, available={}",
            sz,
            self.window_size,
            self.available
        );
        self.window_size -= sz;
        self.available -= sz;
    }

    pub fn send_data(&mut self, sz: WindowSize) {
        tracing::trace!(
            "send_data; sz={}; window={}; available={}",
            sz,
            self.window_size,
            self.available
        );

        assert!(sz <= self.window_size);

        self.window_size -= sz;
        self.available -= sz;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Window(i32);

impl Window {
    pub fn as_size(&self) -> WindowSize {
        if self.0 < 0 {
            0
        } else {
            self.0 as WindowSize
        }
    }

    pub fn checked_size(&self) -> WindowSize {
        assert!(self.0 >= 0, "negative Window");
        self.0 as WindowSize
    }
}

impl PartialEq<WindowSize> for Window {
    fn eq(&self, other: &WindowSize) -> bool {
        if self.0 < 0 {
            false
        } else {
            (self.0 as WindowSize).eq(other)
        }
    }
}

impl PartialEq<Window> for WindowSize {
    fn eq(&self, other: &Window) -> bool {
        other.eq(self)
    }
}

impl PartialOrd<WindowSize> for Window {
    fn partial_cmp(&self, other: &WindowSize) -> Option<::std::cmp::Ordering> {
        if self.0 < 0 {
            Some(::std::cmp::Ordering::Less)
        } else {
            (self.0 as WindowSize).partial_cmp(other)
        }
    }
}

impl PartialOrd<Window> for WindowSize {
    fn partial_cmp(&self, other: &Window) -> Option<::std::cmp::Ordering> {
        if other.0 < 0 {
            Some(::std::cmp::Ordering::Greater)
        } else {
            self.partial_cmp(&(other.0 as WindowSize))
        }
    }
}

impl ::std::ops::SubAssign<WindowSize> for Window {
    fn sub_assign(&mut self, other: WindowSize) {
        self.0 -= other as i32;
    }
}

impl ::std::ops::Add<WindowSize> for Window {
    type Output = Self;
    fn add(self, other: WindowSize) -> Self::Output {
        Window(self.0 + other as i32)
    }
}

impl ::std::ops::AddAssign<WindowSize> for Window {
    fn add_assign(&mut self, other: WindowSize) {
        self.0 += other as i32;
    }
}

impl fmt::Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<Window> for isize {
    fn from(w: Window) -> isize {
        w.0 as isize
    }
}
