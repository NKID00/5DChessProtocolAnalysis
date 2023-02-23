use vergen::{vergen, Config, ShaKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    let git = &mut *config.git_mut();
    *git.skip_if_error_mut() = true;
    *git.sha_kind_mut() = ShaKind::Short;
    vergen(config)?;
    Ok(())
}
