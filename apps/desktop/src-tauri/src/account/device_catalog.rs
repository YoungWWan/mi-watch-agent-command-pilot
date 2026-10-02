//! Product names and transport hints, not an installation allowlist.
//! Sources and capability limits: docs/DEVICE_COMPATIBILITY.md.

pub struct DeviceProfile {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub sar_version: u32,
    pub http_supported: Option<bool>,
}

const PROFILES: &[DeviceProfile] = &[
    DeviceProfile {
        name: "Redmi Watch 5",
        aliases: &[
            "Redmi Watch 5",
            "红米手表5",
            "miwear.watch.o65",
            "miwear.watch.o65w",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Redmi Watch 5 eSIM",
        aliases: &["Redmi Watch 5 eSIM", "miwear.watch.o65m"],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Redmi Watch 4",
        aliases: &[
            "Redmi Watch 4",
            "红米手表4",
            "lchz.watch.n65",
            "lchz.watch.n65gl",
        ],
        sar_version: 1,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "Redmi Watch 6",
        aliases: &[
            "Redmi Watch 6",
            "红米手表6",
            "miwear.watch.p65",
            "miwear.watch.p65gl",
            "miwear.watch.p65gln",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "小米手环 8",
        aliases: &[
            "小米手环8",
            "Xiaomi Smart Band 8",
            "miwear.watch.m66",
            "miwear.watch.m66cn",
            "miwear.watch.m66gl",
        ],
        sar_version: 2,
        http_supported: None,
    },
    DeviceProfile {
        name: "小米手环 8 NFC",
        aliases: &[
            "小米手环8 NFC",
            "Xiaomi Smart Band 8 NFC",
            "miwear.watch.m66nfc",
            "miwear.watch.m66gln",
        ],
        sar_version: 2,
        http_supported: None,
    },
    DeviceProfile {
        name: "小米手环 8 Pro",
        aliases: &[
            "小米手环8 Pro",
            "Xiaomi Smart Band 8 Pro",
            "lchz.watch.m67",
            "lchz.watch.m67ys",
            "lchz.watch.m67gl",
        ],
        sar_version: 1,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 9",
        aliases: &[
            "小米手环9",
            "Xiaomi Smart Band 9",
            "miwear.watch.n66",
            "miwear.watch.n66cn",
            "miwear.watch.n66gl",
        ],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 9 NFC",
        aliases: &[
            "小米手环9 NFC",
            "Xiaomi Smart Band 9 NFC",
            "miwear.watch.n66nfc",
            "miwear.watch.n66gln",
        ],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 9 陶瓷版",
        aliases: &["miwear.watch.n66tc"],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 9 Pro",
        aliases: &[
            "小米手环9 Pro",
            "Xiaomi Smart Band 9 Pro",
            "miwear.watch.n67",
            "miwear.watch.n67cn",
            "miwear.watch.n67gl",
        ],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 9 Active",
        aliases: &[
            "小米手环9 Active",
            "Xiaomi Smart Band 9 Active",
            "miwear.watch.n69",
            "miwear.watch.n69cn",
            "miwear.watch.n69gl",
        ],
        sar_version: 2,
        http_supported: None,
    },
    DeviceProfile {
        name: "小米手环 10",
        aliases: &[
            "小米手环10",
            "Xiaomi Smart Band 10",
            "miwear.watch.o66",
            "miwear.watch.o66cn",
            "miwear.watch.o66gl",
        ],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "小米手环 10 NFC",
        aliases: &[
            "小米手环10 NFC",
            "Xiaomi Smart Band 10 NFC",
            "miwear.watch.o66nfc",
            "miwear.watch.o66gln",
        ],
        sar_version: 2,
        http_supported: Some(false),
    },
    DeviceProfile {
        name: "Xiaomi Watch S1 Pro",
        aliases: &["Xiaomi Watch S1 Pro", "小米手表S1 Pro", "mijia.watch.l61"],
        sar_version: 1,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Xiaomi Watch S3",
        aliases: &[
            "Xiaomi Watch S3",
            "小米手表S3",
            "mijia.watch.n62",
            "mijia.watch.n62lte",
            "mijia.watch.n62w",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Xiaomi Watch S4",
        aliases: &[
            "Xiaomi Watch S4",
            "小米手表S4",
            "mijia.watch.o62",
            "mijia.watch.o62lte",
            "mijia.watch.o62gl",
            "mijia.watch.n62s",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Xiaomi Watch S4 15周年纪念版",
        aliases: &[
            "Xiaomi Watch S4 15th Anniversary",
            "小米手表S4 15周年纪念版",
            "mijia.watch.o62m",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Xiaomi Watch S4 41mm",
        aliases: &[
            "Xiaomi Watch S4 41mm",
            "小米手表S4 41mm",
            "miwear.watch.o63",
            "miwear.watch.o63w",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
    DeviceProfile {
        name: "Xiaomi Watch S5",
        aliases: &[
            "Xiaomi Watch S5",
            "Xiaomi Watch S5 46mm",
            "小米手表S5",
            "miwear.watch.p62",
            "miwear.watch.p62lte",
            "miwear.watch.p62g",
        ],
        sar_version: 2,
        http_supported: Some(true),
    },
];

fn normalize(model: &str) -> String {
    model
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn lookup(model: &str) -> Option<&'static DeviceProfile> {
    let model = normalize(model);
    PROFILES.iter().find(|profile| {
        profile
            .aliases
            .iter()
            .any(|alias| normalize(alias) == model)
    })
}

pub fn display_name(model: &str) -> String {
    if let Some(profile) = lookup(model) {
        return profile.name.to_string();
    }
    let model = model.trim();
    if model.is_empty() || model.eq_ignore_ascii_case("Wearable Device") || model.contains('.') {
        "小米穿戴设备（型号待识别）".to_string()
    } else {
        model.to_string()
    }
}

pub fn is_verified(model: &str) -> bool {
    // Only this product has been validated with our bundled application.
    lookup(model).is_some_and(|profile| profile.name == "Redmi Watch 5")
}

pub fn compatibility_note(model: &str) -> String {
    if is_verified(model) {
        "已在 Redmi Watch 5 验证；其他固件版本仍需实测。".to_string()
    } else if lookup(model).is_some_and(|profile| profile.http_supported == Some(false)) {
        "可尝试安装；界面尚未适配。官方文档未支持本应用使用的 HTTP 请求接口，安装后可能无法收发指令。".to_string()
    } else {
        "可尝试安装；界面与功能尚未验证，实际可用性取决于设备的快应用、蓝牙与网络能力。".to_string()
    }
}

pub fn sar_version(model: &str) -> u32 {
    lookup(model).map_or(2, |profile| profile.sar_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_models_resolve_without_guessing_from_digits() {
        assert_eq!(display_name("miwear.watch.n67cn"), "小米手环 9 Pro");
        assert_eq!(display_name(" miwear.watch.o65 "), "Redmi Watch 5");
        assert_eq!(display_name("lchz.watch.n65"), "Redmi Watch 4");
        assert_eq!(
            display_name("mijia.watch.potential5"),
            "小米穿戴设备（型号待识别）"
        );
        assert_eq!(display_name("Xiaomi Watch S5"), "Xiaomi Watch S5");
        assert_eq!(display_name("Redmi Watch 5 Active"), "Redmi Watch 5 Active");
        assert!(!is_verified("miwear.watch.m66"));
        assert!(!is_verified("Redmi Watch 5 Active"));
        assert!(!is_verified("Redmi Watch 5 eSIM"));
        assert!(is_verified("REDMI Watch 5"));
    }

    #[test]
    fn transport_hints_cover_older_devices_and_unknown_models() {
        assert_eq!(sar_version("lchz.watch.n65"), 1);
        assert_eq!(sar_version("lchz.watch.m67"), 1);
        assert_eq!(sar_version("mijia.watch.l61"), 1);
        assert_eq!(sar_version("miwear.watch.n67cn"), 2);
        assert_eq!(sar_version("miwear.watch.future"), 2);
        assert!(lookup("miwear.watch.o65unknown").is_none());
    }

    #[test]
    fn installation_and_network_support_are_distinct() {
        let note = compatibility_note("miwear.watch.n67cn");
        assert!(note.contains("可尝试安装"));
        assert!(note.contains("HTTP"));
        assert!(compatibility_note("mijia.watch.o62").contains("尚未验证"));
        assert!(compatibility_note("miwear.watch.future").contains("可尝试安装"));
    }
}
