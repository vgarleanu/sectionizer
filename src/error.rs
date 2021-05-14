use err_derive::Error;

#[derive(Error, Debug)]
#[error(display = "An error occured.")]
pub enum SectionizerError {
    #[error(display = "An Error has occured with nightfall")]
    NightfallError(#[error(source)] nightfall::error::NightfallError),
}
