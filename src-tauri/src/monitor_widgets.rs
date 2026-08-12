use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::settings::DIVIDER_PREFIX;

pub type InstanceId = String;

// One placed copy of a widget on the strip. Distinct from a widget_id (e.g. "cpu"),
// which now identifies the widget *kind*, not a placement - see MonitorWidgets.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct StripInstance {
    pub instance_id: InstanceId,
    pub widget_id: String,
}

// Keyed by monitor device name ("" = the same "whatever is primary" convention as
// taskbar_monitor). `transparent` + `default` keep the JSON shape a plain
// `{"monitor": [...]}` map, so a per-monitor round trip needs no wrapper object.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default, transparent)]
pub struct MonitorWidgets(pub HashMap<String, Vec<StripInstance>>);

impl MonitorWidgets {
    #[allow(dead_code)]
    pub fn monitors(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    pub fn instances_for(&self, monitor: &str) -> &[StripInstance] {
        self.0.get(monitor).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn all(&self) -> impl Iterator<Item = &StripInstance> {
        self.0.values().flatten()
    }

    pub fn monitor_of(&self, instance_id: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(_, v)| v.iter().any(|si| si.instance_id == instance_id))
            .map(|(m, _)| m.as_str())
    }

    /// Smallest unused "<widget_id>#n" across ALL monitors. A divider already carries
    /// a unique id (DIVIDER_PREFIX) and is returned unchanged, no "#n" suffix.
    #[allow(dead_code)]
    pub fn next_instance_id(&self, widget_id: &str) -> InstanceId {
        if widget_id.starts_with(DIVIDER_PREFIX) {
            return widget_id.to_string();
        }
        let mut n: u32 = 1;
        loop {
            let candidate = format!("{widget_id}#{n}");
            if !self.all().any(|si| si.instance_id == candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_instance_id_avoids_existing_suffixes() {
        let empty = MonitorWidgets::default();
        assert_eq!(empty.next_instance_id("conductor"), "conductor#1");

        let taken = MonitorWidgets(HashMap::from([(
            String::new(),
            vec![StripInstance { instance_id: "conductor#1".into(), widget_id: "conductor".into() }],
        )]));
        assert_eq!(taken.next_instance_id("conductor"), "conductor#2");
    }
}
