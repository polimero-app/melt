use serde_json::{Map, Value};

use crate::moonraker::{HmsEvent, HmsSeverity};

pub(super) fn events(print: &Map<String, Value>) -> Vec<HmsEvent> {
    print
        .get("hms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let attr = unsigned_word(item.get("attr"))?;
            let code = unsigned_word(item.get("code"))?;
            (attr != 0 || code != 0).then(|| classify(attr, code))
        })
        .collect()
}

fn unsigned_word(value: Option<&Value>) -> Option<u32> {
    value?.as_u64()?.try_into().ok()
}

fn classify(attr: u32, code: u32) -> HmsEvent {
    let severity = match code >> 16 {
        1 => HmsSeverity::Error,
        2 => HmsSeverity::Warning,
        3 => HmsSeverity::Notification,
        _ => HmsSeverity::Unknown,
    };
    let raw_code = format!(
        "{:04X}-{:04X}-{:04X}-{:04X}",
        attr >> 16,
        attr & 0xffff,
        code >> 16,
        code & 0xffff,
    );
    let (name, message, alert) = classification(attr, code, severity);
    HmsEvent {
        code: name,
        message: message.into(),
        raw_code,
        severity,
        alert,
    }
}

fn classification(
    attr: u32,
    code: u32,
    severity: HmsSeverity,
) -> (&'static str, &'static str, bool) {
    if attr == 0x0500_0500 && code == 0x0001_0007 {
        return (
            "command_verification_failed",
            "The printer rejected an MQTT command because command verification failed.",
            false,
        );
    }

    if is_ams_auto_refill_notice(attr, code) {
        return match code {
            0x0003_0001 => (
                "ams_auto_refill_in_progress",
                "The AMS is switching to a backup spool after filament ran out.",
                false,
            ),
            _ => (
                "ams_auto_refill_completed",
                "The AMS switched to a backup spool with matching filament.",
                false,
            ),
        };
    }

    let message_code = code & 0xffff;
    match (attr, message_code) {
        (0x0300_9700, 0x0001) => ("top_cover_open", "The top cover is open.", true),
        (0x0300_9700, 0x0002) => (
            "top_cover_front_right_sensor",
            "The top cover Hall sensor (front right) is abnormal. Check whether its connection wire is loose.",
            true,
        ),
        (0x0300_9700, 0x0003) => (
            "top_cover_rear_left_sensor",
            "The top cover Hall sensor (rear left) is abnormal. Check whether its connection wire is loose.",
            true,
        ),
        (0x0300_9700, 0x0004) => (
            "top_laser_protection_plate_missing",
            "The top laser protection plate is not detected. Install it according to the Bambu Lab Wiki, then restart the task.",
            true,
        ),
        _ => (
            "hardware_error",
            "The printer reported a hardware event.",
            severity != HmsSeverity::Notification,
        ),
    }
}

fn is_ams_auto_refill_notice(attr: u32, code: u32) -> bool {
    let module = attr >> 24;
    let slot = (attr >> 8) & 0xff;
    matches!(module, 0x07 | 0x18)
        && (0x20..=0x23).contains(&slot)
        && matches!(code, 0x0003_0001 | 0x0003_0002)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(attr: u32, code: u32) -> HmsEvent {
        let report = json!({ "hms": [{ "attr": attr, "code": code }] });
        events(report.as_object().unwrap()).remove(0)
    }

    #[test]
    fn classifies_command_verification_as_diagnostic_only() {
        let event = event(0x0500_0500, 0x0001_0007);

        assert_eq!(event.code, "command_verification_failed");
        assert_eq!(event.raw_code, "0500-0500-0001-0007");
        assert_eq!(event.severity, HmsSeverity::Error);
        assert!(!event.alert);
    }

    #[test]
    fn classifies_only_slot_scoped_ams_refill_notifications() {
        for attr in [0x0700_2000, 0x0700_2300, 0x1800_2000, 0x1800_2300] {
            assert!(!event(attr, 0x0003_0001).alert);
            assert!(!event(attr, 0x0003_0002).alert);
        }

        assert_eq!(event(0x0700_2400, 0x0003_0001).code, "hardware_error");
        assert_eq!(event(0x0800_2000, 0x0003_0001).code, "hardware_error");
        assert!(event(0x0700_2000, 0x0001_0001).alert);
    }

    #[test]
    fn preserves_unknown_notifications_without_alerting() {
        let event = event(0x1234_5678, 0x0003_0099);

        assert_eq!(event.code, "hardware_error");
        assert_eq!(event.raw_code, "1234-5678-0003-0099");
        assert_eq!(event.severity, HmsSeverity::Notification);
        assert!(!event.alert);
    }
}
