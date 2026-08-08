use std::fmt;
use std::io;

#[derive(Debug)]
pub(super) enum PlatformError {
    Context {
        context: String,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    Related {
        context: &'static str,
        sources: Box<RelatedErrorSource>,
    },
}

#[derive(Debug)]
pub(super) struct RelatedErrorSource {
    error: PlatformError,
    next: Option<Box<RelatedErrorSource>>,
}

pub(super) type PlatformResult<T> = Result<T, PlatformError>;

impl PlatformError {
    pub(super) fn message(context: impl Into<String>) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: None,
        }
    }

    pub(super) fn io(context: impl Into<String>, source: io::Error) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }

    pub(super) fn with_source(context: impl Into<String>, source: PlatformError) -> Self {
        PlatformError::Context {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }

    pub(super) fn chain(mut errors: Vec<PlatformError>) -> Self {
        if errors.len() <= 1 {
            return errors
                .pop()
                .unwrap_or_else(|| PlatformError::message("unknown platform error"));
        }
        let mut sources = None;
        while let Some(error) = errors.pop() {
            sources = Some(Box::new(RelatedErrorSource {
                error,
                next: sources,
            }));
        }
        PlatformError::Related {
            context: "multiple platform errors",
            sources: sources.expect("multiple errors produced at least one source"),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::Context { context, source } => match source {
                Some(source) => write!(formatter, "{}: {}", context, source),
                None => formatter.write_str(context),
            },
            PlatformError::Related { context, sources } => {
                write!(formatter, "{}: {}", context, sources)
            }
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PlatformError::Context { source, .. } => source
                .as_deref()
                .map(|source| source as &(dyn std::error::Error + 'static)),
            PlatformError::Related { sources, .. } => Some(sources),
        }
    }
}

impl fmt::Display for RelatedErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.error)?;
        if let Some(next) = &self.next {
            write!(formatter, "; {}", next)?;
        }
        Ok(())
    }
}

impl std::error::Error for RelatedErrorSource {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.next
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}
