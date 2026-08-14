"""Make the connectivity fake's trait reads consume their ordered queues first."""

import pathlib

path = pathlib.Path(
    "/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/connectivity/fake.rs"
)
src = path.read_text(encoding="utf-8")

REPLACEMENTS = [
    # radio
    (
        """        self.guard_reads()?;
        Ok(self.radio_enabled)
    }""",
        """        self.guard_reads()?;
        let mut queue = self.radio_queue.lock().expect("radio queue");
        if queue.is_empty() {
            // Never scripted as a queue: fall back to the single-value style.
            return Ok(self.radio_enabled);
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("radio state"))
    }""",
    ),
    # active ssid
    (
        """        self.guard_reads()?;
        Ok(self.active_ssid.clone())
    }""",
        """        self.guard_reads()?;
        let mut queue = self.ssid_queue.lock().expect("ssid queue");
        if queue.is_empty() {
            return Ok(self.active_ssid.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("active ssid"))
    }""",
    ),
    # scan
    (
        """        self.guard_reads()?;
        Ok(self.networks.clone())
    }""",
        """        self.guard_reads()?;
        let mut queue = self.scan_queue.lock().expect("scan queue");
        if queue.is_empty() {
            return Ok(self.networks.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("wifi scan"))
    }""",
    ),
    # list profiles
    (
        """        self.guard_reads()?;
        Ok(self.profiles.clone())
    }""",
        """        self.guard_reads()?;
        let mut queue = self.profiles_queue.lock().expect("profiles queue");
        if queue.is_empty() {
            return Ok(self.profiles.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("profile listing"))
    }""",
    ),
    # device connected
    (
        """        self.guard_reads()?;
        Ok(self
            .devices
            .iter()
            .any(|d| d.name == device.as_str() && d.is_connected()))
    }""",
        """        self.guard_reads()?;
        let mut queue = self.device_connected_queue.lock().expect("device queue");
        if queue.is_empty() {
            return Ok(self
                .devices
                .iter()
                .any(|d| d.name == device.as_str() && d.is_connected()));
        }
        match queue.pop_front() {
            Some(Ok(connected)) => Ok(connected),
            // An UNKNOWN device state, deliberately distinct from `false`.
            Some(Err(_label)) => Err(self.queue_exhausted("device connected state")),
            None => Err(self.queue_exhausted("device connected state")),
        }
    }""",
    ),
    # profile saved
    (
        """        self.guard_reads()?;
        Ok(self
            .profiles
            .iter()
            .any(|p| p.uuid == profile.as_str() || p.name == profile.as_str()))
    }""",
        """        self.guard_reads()?;
        let mut queue = self.profile_saved_queue.lock().expect("profile saved queue");
        if queue.is_empty() {
            return Ok(self
                .profiles
                .iter()
                .any(|p| p.uuid == profile.as_str() || p.name == profile.as_str()));
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("profile saved state"))
    }""",
    ),
    # active profile
    (
        """        self.guard_reads()?;
        match device {
            // Device-scoped: only a profile bound to that device counts.
            Some(dev) => Ok(self
                .profiles
                .iter()
                .find(|p| p.device.as_deref() == Some(dev.as_str()))
                .map(|p| NetworkProfileId::new(&p.uuid))),
            None => Ok(self.active_profile.clone()),
        }
    }""",
        """        self.guard_reads()?;
        {
            let mut queue = self.active_profile_queue.lock().expect("active profile queue");
            if !queue.is_empty() {
                return queue
                    .pop_front()
                    .ok_or_else(|| self.queue_exhausted("active profile"));
            }
        }
        match device {
            // Device-scoped: only a profile bound to that device counts.
            Some(dev) => Ok(self
                .profiles
                .iter()
                .find(|p| p.device.as_deref() == Some(dev.as_str()))
                .map(|p| NetworkProfileId::new(&p.uuid))),
            None => Ok(self.active_profile.clone()),
        }
    }""",
    ),
    # dispatch honours the scripted outcome
    (
        """        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }""",
        """        if let Some(outcome) = self.dispatch_outcome.lock().expect("dispatch outcome").clone() {
            return Ok(outcome);
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }""",
    ),
]

for old, new in REPLACEMENTS:
    if old not in src:
        raise SystemExit("MISS:\n" + old[:120])
    src = src.replace(old, new, 1)

# VecDeque import
if "use std::collections::VecDeque;" not in src:
    src = src.replace(
        "use std::collections::HashMap;",
        "use std::collections::{HashMap, VecDeque};",
        1,
    )

path.write_text(src, encoding="utf-8")
print("connectivity fake reads are queue-aware")
