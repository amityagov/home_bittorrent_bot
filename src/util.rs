pub trait ResultExt<T, E> {
    fn log_error(self) -> Result<T, E>;
}

impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: std::fmt::Debug,
{
    fn log_error(self) -> Result<T, E> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                log::error!("{:?}", err);
                Err(err)
            }
        }
    }
}
