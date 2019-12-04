use cargo_platform::{Cfg, CfgExpr, Platform};
use std::fmt;
use std::str::FromStr;

macro_rules! c {
    ($a:ident) => {
        Cfg::Name(stringify!($a).to_string())
    };
    ($a:ident = $e:expr) => {
        Cfg::KeyPair(stringify!($a).to_string(), $e.to_string())
    };
}

macro_rules! e {
    (any($($t:tt),*)) => (CfgExpr::Any(vec![$(e!($t)),*]));
    (all($($t:tt),*)) => (CfgExpr::All(vec![$(e!($t)),*]));
    (not($($t:tt)*)) => (CfgExpr::Not(Box::new(e!($($t)*))));
    (($($t:tt)*)) => (e!($($t)*));
    ($($t:tt)*) => (CfgExpr::Value(c!($($t)*)));
}

fn good<T>(s: &str, expected: T)
where
    T: FromStr + PartialEq + fmt::Debug,
    T::Err: fmt::Display,
{
    let c = match T::from_str(s) {
        Ok(c) => c,
        Err(e) => panic!("failed to parse `{}`: {}", s, e),
    };
    assert_eq!(c, expected);
}

fn bad<T>(s: &str, err: &str)
where
    T: FromStr + fmt::Display,
    T::Err: fmt::Display,
{
    let e = match T::from_str(s) {
        Ok(cfg) => panic!("expected `{}` to not parse but got {}", s, cfg),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains(err),
        "when parsing `{}`,\n\"{}\" not contained \
         inside: {}",
        s,
        err,
        e
    );
}

#[test]
fn cfg_syntax() {
    good("foo", c!(foo));
    good("_bar", c!(_bar));
    good(" foo", c!(foo));
    good(" foo  ", c!(foo));
    good(" foo  = \"bar\"", c!(foo = "bar"));
    good("foo=\"\"", c!(foo = ""));
    good(" foo=\"3\"      ", c!(foo = "3"));
    good("foo = \"3 e\"", c!(foo = "3 e"));
}

#[test]
fn cfg_syntax_bad() {
    bad::<Cfg>("", "but cfg expression ended");
    bad::<Cfg>(" ", "but cfg expression ended");
    bad::<Cfg>("\t", "unexpected character");
    bad::<Cfg>("7", "unexpected character");
    bad::<Cfg>("=", "expected identifier");
    bad::<Cfg>(",", "expected identifier");
    bad::<Cfg>("(", "expected identifier");
    bad::<Cfg>("foo (", "unexpected content `(` found after cfg expression");
    bad::<Cfg>("bar =", "expected a string");
    bad::<Cfg>("bar = \"", "unterminated string");
    bad::<Cfg>(
        "foo, bar",
        "unexpected content `, bar` found after cfg expression",
    );
}

#[test]
fn cfg_expr() {
    good("foo", e!(foo));
    good("_bar", e!(_bar));
    good(" foo", e!(foo));
    good(" foo  ", e!(foo));
    good(" foo  = \"bar\"", e!(foo = "bar"));
    good("foo=\"\"", e!(foo = ""));
    good(" foo=\"3\"      ", e!(foo = "3"));
    good("foo = \"3 e\"", e!(foo = "3 e"));

    good("all()", e!(all()));
    good("all(a)", e!(all(a)));
    good("all(a, b)", e!(all(a, b)));
    good("all(a, )", e!(all(a)));
    good("not(a = \"b\")", e!(not(a = "b")));
    good("not(all(a))", e!(not(all(a))));
}

#[test]
fn cfg_expr_bad() {
    bad::<CfgExpr>(" ", "but cfg expression ended");
    bad::<CfgExpr>(" all", "expected `(`");
    bad::<CfgExpr>("all(a", "expected `)`");
    bad::<CfgExpr>("not", "expected `(`");
    bad::<CfgExpr>("not(a", "expected `)`");
    bad::<CfgExpr>("a = ", "expected a string");
    bad::<CfgExpr>("all(not())", "expected identifier");
    bad::<CfgExpr>(
        "foo(a)",
        "unexpected content `(a)` found after cfg expression",
    );
}

#[test]
fn cfg_matches() {
    assert!(e!(foo).matches(&[c!(bar), c!(foo), c!(baz)]));
    assert!(e!(any(foo)).matches(&[c!(bar), c!(foo), c!(baz)]));
    assert!(e!(any(foo, bar)).matches(&[c!(bar)]));
    assert!(e!(any(foo, bar)).matches(&[c!(foo)]));
    assert!(e!(all(foo, bar)).matches(&[c!(foo), c!(bar)]));
    assert!(e!(all(foo, bar)).matches(&[c!(foo), c!(bar)]));
    assert!(e!(not(foo)).matches(&[c!(bar)]));
    assert!(e!(not(foo)).matches(&[]));
    assert!(e!(any((not(foo)), (all(foo, bar)))).matches(&[c!(bar)]));
    assert!(e!(any((not(foo)), (all(foo, bar)))).matches(&[c!(foo), c!(bar)]));

    assert!(!e!(foo).matches(&[]));
    assert!(!e!(foo).matches(&[c!(bar)]));
    assert!(!e!(foo).matches(&[c!(fo)]));
    assert!(!e!(any(foo)).matches(&[]));
    assert!(!e!(any(foo)).matches(&[c!(bar)]));
    assert!(!e!(any(foo)).matches(&[c!(bar), c!(baz)]));
    assert!(!e!(all(foo)).matches(&[c!(bar), c!(baz)]));
    assert!(!e!(all(foo, bar)).matches(&[c!(bar)]));
    assert!(!e!(all(foo, bar)).matches(&[c!(foo)]));
    assert!(!e!(all(foo, bar)).matches(&[]));
    assert!(!e!(not(bar)).matches(&[c!(bar)]));
    assert!(!e!(not(bar)).matches(&[c!(baz), c!(bar)]));
    assert!(!e!(any((not(foo)), (all(foo, bar)))).matches(&[c!(foo)]));
}

#[test]
fn bad_target_name() {
    bad::<Platform>(
        "any(cfg(unix), cfg(windows))",
        "failed to parse `any(cfg(unix), cfg(windows))` as a cfg expression: \
         invalid target specifier: unexpected `(` character, \
         cfg expressions must start with `cfg(`",
    );
    bad::<Platform>(
        "!foo",
        "failed to parse `!foo` as a cfg expression: \
         invalid target specifier: unexpected character ! in target name",
    );
    bad::<Platform>(
        "cfg(debug_assertions)",
        "failed to parse `debug_assertions` as a cfg expression: \
         invalid name in target cfg: debug_assertions",
    );
    bad::<Platform>(
        "cfg(feature = \"abc\")",
        "failed to parse `feature = \"abc\"` as a cfg expression: \
         invalid key in target cfg: feature",
    );
}

#[test]
fn round_trip_platform() {
    fn rt(s: &str) {
        let p = Platform::from_str(s).unwrap();
        let s2 = p.to_string();
        let p2 = Platform::from_str(&s2).unwrap();
        assert_eq!(p, p2);
    }
    rt("x86_64-apple-darwin");
    rt("foo");
    rt("cfg(windows)");
    rt("cfg(target_os = \"windows\")");
    rt(
        "cfg(any(all(any(target_os = \"android\", target_os = \"linux\"), \
         any(target_arch = \"aarch64\", target_arch = \"arm\", target_arch = \"powerpc64\", \
         target_arch = \"x86\", target_arch = \"x86_64\")), \
         all(target_os = \"freebsd\", target_arch = \"x86_64\")))",
    );
}
