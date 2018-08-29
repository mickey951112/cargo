use std::fmt;
use std::marker;

pub type MatchResult = Result<(), String>;

pub trait Matcher<T>: fmt::Debug {
    fn matches(&self, actual: T) -> Result<(), String>;
}

pub fn assert_that<T, U: Matcher<T>>(actual: T, matcher: U) {
    if let Err(e) = matcher.matches(actual) {
        panic!("\nExpected: {:?}\n    but: {}", matcher, e)
    }
}

pub fn is_not<T, M: Matcher<T>>(matcher: M) -> IsNot<T, M> {
    IsNot {
        matcher,
        _marker: marker::PhantomData,
    }
}

#[derive(Debug)]
pub struct IsNot<T, M> {
    matcher: M,
    _marker: marker::PhantomData<T>,
}

impl<T, M: Matcher<T>> Matcher<T> for IsNot<T, M>
where
    T: fmt::Debug,
{
    fn matches(&self, actual: T) -> Result<(), String> {
        match self.matcher.matches(actual) {
            Ok(_) => Err("matched".to_string()),
            Err(_) => Ok(()),
        }
    }
}
