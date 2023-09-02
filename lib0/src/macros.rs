// This code is based on serde_json::json! macro (see: https://docs.rs/serde_json/latest/src/serde_json/macros.rs.html#53-58).
// Kudos to the original authors.

/// Construct a lib0 [Any] value literal.
///
/// # Examples
///
/// ```rust
///
/// use lib0::any;
///
/// let value = any!({
///   "code": 200,
///   "success": true,
///   "payload": {
///     "features": [
///       "lib0",
///       true
///     ]
///   }
/// });
/// ```
#[macro_export(local_inner_macros)]
macro_rules! any {
    // Hide distracting implementation details from the generated rustdoc.
    ($($any:tt)+) => {
        any_internal!($($any)+)
    };
}

#[macro_export(local_inner_macros)]
#[doc(hidden)]
macro_rules! any_internal {
    (@array [$($items:expr,)*]) => {
        any_internal_array![$($items,)*]
    };

    // Done without trailing comma.
    (@array [$($items:expr),*]) => {
        any_internal_array![$($items),*]
    };

    // Next item is `null`.
    (@array [$($items:expr,)*] null $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(null)] $($rest)*)
    };

    // Next item is `true`.
    (@array [$($items:expr,)*] true $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(true)] $($rest)*)
    };

    // Next item is `false`.
    (@array [$($items:expr,)*] false $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!(false)] $($rest)*)
    };

    // Next item is an array.
    (@array [$($items:expr,)*] [$($array:tt)*] $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!([$($array)*])] $($rest)*)
    };

    // Next item is a map.
    (@array [$($items:expr,)*] {$($map:tt)*} $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!({$($map)*})] $($rest)*)
    };

    // Next item is an expression followed by comma.
    (@array [$($items:expr,)*] $next:expr, $($rest:tt)*) => {
        any_internal!(@array [$($items,)* any_internal!($next),] $($rest)*)
    };

    // Last item is an expression with no trailing comma.
    (@array [$($items:expr,)*] $last:expr) => {
        any_internal!(@array [$($items,)* any_internal!($last)])
    };

    // Comma after the most recent item.
    (@array [$($items:expr),*] , $($rest:tt)*) => {
        any_internal!(@array [$($items,)*] $($rest)*)
    };

    // Unexpected token after most recent item.
    (@array [$($items:expr),*] $unexpected:tt $($rest:tt)*) => {
        any_unexpected!($unexpected)
    };

    (@object $object:ident () () ()) => {};

    // Insert the current entry followed by trailing comma.
    (@object $object:ident [$($key:tt)+] ($value:expr) , $($rest:tt)*) => {
        let _ = $object.insert(($($key)+).into(), $value);
        any_internal!(@object $object () ($($rest)*) ($($rest)*));
    };

    // Current entry followed by unexpected token.
    (@object $object:ident [$($key:tt)+] ($value:expr) $unexpected:tt $($rest:tt)*) => {
        any_unexpected!($unexpected);
    };

    // Insert the last entry without trailing comma.
    (@object $object:ident [$($key:tt)+] ($value:expr)) => {
        let _ = $object.insert(($($key)+).into(), $value);
    };

    // Next value is `null`.
    (@object $object:ident ($($key:tt)+) (: null $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(null)) $($rest)*);
    };

    // Next value is `true`.
    (@object $object:ident ($($key:tt)+) (: true $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(true)) $($rest)*);
    };

    // Next value is `false`.
    (@object $object:ident ($($key:tt)+) (: false $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!(false)) $($rest)*);
    };

    // Next value is an array.
    (@object $object:ident ($($key:tt)+) (: [$($array:tt)*] $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!([$($array)*])) $($rest)*);
    };

    // Next value is a map.
    (@object $object:ident ($($key:tt)+) (: {$($map:tt)*} $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!({$($map)*})) $($rest)*);
    };

    // Next value is an expression followed by comma.
    (@object $object:ident ($($key:tt)+) (: $value:expr , $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!($value)) , $($rest)*);
    };

    // Last value is an expression with no trailing comma.
    (@object $object:ident ($($key:tt)+) (: $value:expr) $copy:tt) => {
        any_internal!(@object $object [$($key)+] (any_internal!($value)));
    };

    // Missing value for last entry. Trigger a reasonable error message.
    (@object $object:ident ($($key:tt)+) (:) $copy:tt) => {
        // "unexpected end of macro invocation"
        any_internal!();
    };

    // Missing colon and value for last entry. Trigger a reasonable error
    // message.
    (@object $object:ident ($($key:tt)+) () $copy:tt) => {
        // "unexpected end of macro invocation"
        any_internal!();
    };

    // Misplaced colon. Trigger a reasonable error message.
    (@object $object:ident () (: $($rest:tt)*) ($colon:tt $($copy:tt)*)) => {
        // Takes no arguments so "no rules expected the token `:`".
        any_unexpected!($colon);
    };

    // Found a comma inside a key. Trigger a reasonable error message.
    (@object $object:ident ($($key:tt)*) (, $($rest:tt)*) ($comma:tt $($copy:tt)*)) => {
        // Takes no arguments so "no rules expected the token `,`".
        any_unexpected!($comma);
    };

    // Key is fully parenthesized. This avoids clippy double_parens false
    // positives because the parenthesization may be necessary here.
    (@object $object:ident () (($key:expr) : $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object ($key) (: $($rest)*) (: $($rest)*));
    };

    // Refuse to absorb colon token into key expression.
    (@object $object:ident ($($key:tt)*) (: $($unexpected:tt)+) $copy:tt) => {
        json_expect_expr_comma!($($unexpected)+);
    };

    // Munch a token into the current key.
    (@object $object:ident ($($key:tt)*) ($tt:tt $($rest:tt)*) $copy:tt) => {
        any_internal!(@object $object ($($key)* $tt) ($($rest)*) ($($rest)*));
    };

    //////////////////////////////////////////////////////////////////////////
    // The main implementation.
    //
    // Must be invoked as: any_internal!($($json)+)
    //////////////////////////////////////////////////////////////////////////

    (null) => {
        $crate::any::Any::Null
    };

    (true) => {
        $crate::any::Any::Bool(true)
    };

    (false) => {
        $crate::any::Any::Bool(false)
    };

    ([]) => {
        $crate::any::Any::Array(any_internal_array![])
    };

    ([ $($tt:tt)+ ]) => {
        $crate::any::Any::Array(any_internal!(@array [] $($tt)+))
    };

    ({}) => {
        $crate::any::Any::Map(std::boxed::Box::new(std::collections::HashMap::new()))
    };

    ({ $($tt:tt)+ }) => {
        $crate::any::Any::Map({
            let mut object = std::boxed::Box::new(std::collections::HashMap::new());
            any_internal!(@object object () ($($tt)+) ($($tt)+));
            object
        })
    };

    // Any Serialize type: numbers, strings, struct literals, variables etc.
    // Must be below every other rule.
    ($other:expr) => {
        ($other).into()
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_internal_array {
    ($($content:tt)*) => {
        std::boxed::Box::from([$($content)*])
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_unexpected {
    () => {};
}

#[macro_export]
#[doc(hidden)]
macro_rules! any_expect_expr_comma {
    ($e:expr , $($tt:tt)*) => {};
}
