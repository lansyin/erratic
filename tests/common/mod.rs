#![allow(dead_code)]

use std::{
    error,
    fmt::{self, Display},
};

#[derive(Debug)]
pub struct TestError(pub &'static str);

impl Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for TestError {}

#[derive(Debug, PartialEq)]
pub enum TestState {
    FileNotFound,
    PermissionDenied,
}

#[derive(Debug, PartialEq)]
pub struct TestMessage(pub &'static str);

impl Display for TestMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self.0, f)
    }
}
