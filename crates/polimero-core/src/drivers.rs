use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DriverInfo {
    pub name: &'static str,
    pub description: &'static str,
}

pub fn registered() -> [DriverInfo; 2] {
    [
        DriverInfo {
            name: "bambu-lan",
            description: "Bambu Lab printers over LAN mode",
        },
        DriverInfo {
            name: "moonraker",
            description: "Moonraker-compatible Klipper printers",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_drivers_match_the_current_product() {
        assert_eq!(
            registered().map(|driver| driver.name),
            ["bambu-lan", "moonraker"]
        );
    }
}
