#[derive(Clone, Debug)]
pub struct Error {
    inner: String,
    backtrace: Option<Backtrace>,
}

impl Error {
    #[allow(dead_code)]
    pub(crate) fn new(inner: String) -> Self {
        Self {
            inner,
            backtrace: Backtrace::new(),
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.inner)?;
        if let Some(backtrace) = self.backtrace.as_ref() {
            writeln!(f)?;
            writeln!(f, "Backtrace:")?;
            writeln!(f, "{}", backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl<'s> From<&'s str> for Error {
    fn from(other: &'s str) -> Self {
        Self::new(other.into())
    }
}

impl<'s> From<&'s String> for Error {
    fn from(other: &'s String) -> Self {
        Self::new(other.clone())
    }
}

impl From<String> for Error {
    fn from(other: String) -> Self {
        Self::new(other)
    }
}

#[cfg(feature = "debug")]
#[derive(Debug, Clone)]
struct Backtrace(backtrace::Backtrace);

#[cfg(feature = "debug")]
impl Backtrace {
    fn new() -> Option<Self> {
        Some(Self(backtrace::Backtrace::new()))
    }
}

#[cfg(feature = "debug")]
impl std::fmt::Display for Backtrace {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // `backtrace::Backtrace` uses `Debug` instead of `Display`
        write!(f, "{:?}", self.0)
    }
}

#[cfg(not(feature = "debug"))]
#[derive(Debug, Copy, Clone)]
struct Backtrace;

#[cfg(not(feature = "debug"))]
impl Backtrace {
    fn new() -> Option<Self> {
        None
    }
}

#[cfg(not(feature = "debug"))]
impl std::fmt::Display for Backtrace {
    fn fmt(&self, _: &mut std::fmt::Formatter) -> std::fmt::Result {
        Ok(())
    }
}
