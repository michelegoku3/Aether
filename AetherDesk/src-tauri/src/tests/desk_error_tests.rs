use crate::core::error::{DeskError, DeskResult};

#[test]
fn test_desk_error_display_and_conversion() {
    let err = DeskError::invalid_input("parameter cannot be empty");
    assert_eq!(err.to_string(), "parameter cannot be empty");
    let err_str: String = err.into();
    assert_eq!(err_str, "parameter cannot be empty");

    let not_found = DeskError::not_found("game 730");
    assert_eq!(not_found.to_string(), "Not found: game 730");

    let io_err = DeskError::io("reading appmanifest", std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"));
    assert!(io_err.to_string().contains("reading appmanifest"));
    assert!(io_err.to_string().contains("file not found"));

    let net_err = DeskError::network("https://api.example.com", "connection reset");
    assert_eq!(net_err.to_string(), "Network error on https://api.example.com: connection reset");

    let steam_err = DeskError::steam("Steam process not running");
    assert_eq!(steam_err.to_string(), "Steam error: Steam process not running");

    let prov_err = DeskError::provider("Hubcap", "rate limit exceeded");
    assert_eq!(prov_err.to_string(), "Hubcap error: rate limit exceeded");

    let exec_err = DeskError::execution("Steamless", "exit code 1");
    assert_eq!(exec_err.to_string(), "Steamless execution failed: exit code 1");
}

#[test]
fn test_desk_result_propagation() {
    fn inner_op(succeed: bool) -> DeskResult<u32> {
        if succeed {
            Ok(42)
        } else {
            Err(DeskError::invalid_input("failed operation"))
        }
    }

    fn command_wrapper(succeed: bool) -> Result<u32, String> {
        let val = inner_op(succeed)?;
        Ok(val)
    }

    assert_eq!(command_wrapper(true), Ok(42));
    assert_eq!(command_wrapper(false), Err("failed operation".to_string()));
}
