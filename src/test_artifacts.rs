#![allow(dead_code)]

use core::{
    error,
    fmt::{self, Display},
};

#[derive(Debug, PartialEq, Eq)]
pub struct TestError(pub &'static str);

impl TestError {
    pub const FOO: Self = Self("foo");
    pub const BAR: Self = Self("bar");
    pub const BAZ: Self = Self("baz");
}

impl Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for TestError {}

#[derive(Debug, PartialEq)]
pub enum TestState {
    AppleNotFound,
    BananaDenied,
    CherryUnavailable,
}

#[derive(Debug, PartialEq)]
pub struct TestMessage(pub &'static str);

impl TestMessage {
    pub const HOGE: Self = Self("hoge");
    pub const FUGA: Self = Self("fuga");
    pub const PIYO: Self = Self("piyo");
}

impl Display for TestMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self.0, f)
    }
}
