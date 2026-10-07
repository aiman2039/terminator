//! Build default and explicit recovery override. No automatic retries.
use anyhow::Result;
use eframe::Renderer;

pub fn selected(name: Option<&str>) -> Result<Renderer> {
    if let Some(name) = name {
        return name
            .parse()
            .map_err(|error: String| anyhow::anyhow!("Invalid TERMINATOR_RENDERER: {error}"));
    }
    #[cfg(feature = "wgpu")]
    {
        Ok(Renderer::Wgpu)
    }
    #[cfg(not(feature = "wgpu"))]
    {
        Ok(Renderer::Glow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_glow_selects_the_recovery_renderer() -> Result<()> {
        assert_eq!(selected(Some("glow"))?, Renderer::Glow);
        Ok(())
    }

    #[test]
    fn an_invalid_override_fails_instead_of_changing_renderer() {
        assert!(selected(Some("invalid")).is_err());
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn supported_wgpu_builds_default_to_wgpu() -> Result<()> {
        assert_eq!(selected(None)?, Renderer::Wgpu);
        assert_eq!(selected(Some("wgpu"))?, Renderer::Wgpu);
        Ok(())
    }

    #[cfg(not(feature = "wgpu"))]
    #[test]
    fn builds_without_the_wgpu_feature_keep_glow() -> Result<()> {
        assert_eq!(selected(None)?, Renderer::Glow);
        assert!(selected(Some("wgpu")).is_err());
        Ok(())
    }
}
