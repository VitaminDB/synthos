use synassist::tools::web::action::ArgError;

use crate::agent::tools::executor::ToolError;

pub async fn run(args_json: &str) -> Result<String, ToolError> {
    synassist::tools::web::run(args_json).await.map_err(|e| match e {
        ArgError::BadArgs(s) => ToolError::BadArgs(s),
        ArgError::MissingField(f) => ToolError::MissingField(f),
    })
}
