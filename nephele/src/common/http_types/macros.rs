#[macro_export]
macro_rules! bail {
    ($msg:literal $(,)?) => {
        return $crate::common::http_types::private::Err($crate::format_err!($msg));
    };
    ($msg:expr $(,)?) => {
        return $crate::common::http_types::private::Err($crate::format_err!($msg));
    };
    ($msg:expr, $($arg:tt)*) => {
        return $crate::common::http_types::private::Err($crate::format_err!($msg, $($arg)*));
    };
}

#[macro_export]
macro_rules! ensure {
    ($cond:expr, $msg:literal $(,)?) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err!($msg));
        }
    };
    ($cond:expr, $msg:expr $(,)?) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err!($msg));
        }
    };
    ($cond:expr, $msg:expr, $($arg:tt)*) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err!($msg, $($arg)*));
        }
    };
}

#[macro_export]
macro_rules! ensure_eq {
    ($left:expr, $right:expr, $msg:literal $(,)?) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err!($msg));
        }
    };
    ($left:expr, $right:expr, $msg:expr $(,)?) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err!($msg));
        }
    };
    ($left:expr, $right:expr, $msg:expr, $($arg:tt)*) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err!($msg, $($arg)*));
        }
    };
}

#[macro_export]
macro_rules! format_err {
    ($msg:literal $(,)?) => {
        $crate::common::http_types::private::new_adhoc($msg)
    };
    ($err:expr $(,)?) => ({
        let error = $err;
        Error::new_adhoc(error)
    });
    ($fmt:expr, $($arg:tt)*) => {
        $crate::common::http_types::private::new_adhoc(format!($fmt, $($arg)*))
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! bail_status {
    ($status:literal, $msg:literal $(,)?) => {{
        return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
    }};
    ($status:literal, $msg:expr $(,)?) => {
        return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
    };
    ($status:literal, $msg:expr, $($arg:tt)*) => {
        return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg, $($arg)*));
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! ensure_status {
    ($cond:expr, $status:literal, $msg:literal $(,)?) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
        }
    };
    ($cond:expr, $status:literal, $msg:expr $(,)?) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
        }
    };
    ($cond:expr, $status:literal, $msg:expr, $($arg:tt)*) => {
        if !$cond {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg, $($arg)*));
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! ensure_eq_status {
    ($left:expr, $right:expr, $status:literal, $msg:literal $(,)?) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
        }
    };
    ($left:expr, $right:expr, $status:literal, $msg:expr $(,)?) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg));
        }
    };
    ($left:expr, $right:expr, $status:literal, $msg:expr, $($arg:tt)*) => {
        if $left != $right {
            return $crate::common::http_types::private::Err($crate::format_err_status!($status, $msg, $($arg)*));
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! format_err_status {
    ($status:literal, $msg:literal $(,)?) => {{
        let mut err = $crate::common::http_types::private::new_adhoc($msg);
        err.set_status($status);
        err
    }};
    ($status:literal, $msg:expr $(,)?) => {{
        let mut err = $crate::common::http_types::private::new_adhoc($msg);
        err.set_status($status);
        err
    }};
    ($status:literal, $msg:expr, $($arg:tt)*) => {{
        let mut err = $crate::common::http_types::private::new_adhoc(format!($msg, $($arg)*));
        err.set_status($status);
        err
    }};
}
