use eyre::Result;
use futures::Future;
use tracing::error;

pub(crate) trait Log {
    type Output;
    fn log(self) -> Option<Self::Output>;
}

impl<O> Log for Result<O> {
    type Output = O;

    fn log(self) -> Option<O> {
        match self {
            Ok(v) => Some(v),
            Err(e) => {
                error!("{e:?}");
                None
            }
        }
    }
}

pub(crate) trait LogFuture {
    type Output;

    async fn log(self) -> Option<Self::Output>;
}

impl<T, O> LogFuture for T
where
    T: Future<Output = Result<O>>,
{
    type Output = O;

    async fn log(self) -> Option<O> {
        self.await.log()
    }
}

/// Helper function to create an `Ok` result.
/// This is useful inside closuures where the compiler
/// can't infer the type of the result.
pub fn ok<T>(t: T) -> Result<T> {
    Result::Ok(t)
}
