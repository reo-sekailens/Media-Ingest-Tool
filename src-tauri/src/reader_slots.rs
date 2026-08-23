//! Product-specific reader topology with evidence-backed calibration.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReaderFingerprint {
    pub vendor_id: String,
    pub product_id: String,
    pub reader_serial: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotKind {
    Sd,
    MicroSd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotCalibration {
    pub reader_vendor_id: String,
    pub reader_product_id: String,
    pub logical_unit: u8,
    pub slot_kind: SlotKind,
    pub evidence_note: String,
}

pub fn is_sandisk_pro_reader(reader: &ReaderFingerprint) -> bool {
    // SanDisk's USB vendor ID. Product IDs are intentionally not treated as a
    // slot contract without a captured calibration for that hardware revision.
    reader.vendor_id.eq_ignore_ascii_case("0781")
}

pub fn resolve_slot(
    reader: &ReaderFingerprint,
    logical_unit: u8,
    calibrations: &[SlotCalibration],
) -> Option<SlotKind> {
    calibrations
        .iter()
        .find(|calibration| {
            calibration
                .reader_vendor_id
                .eq_ignore_ascii_case(&reader.vendor_id)
                && calibration
                    .reader_product_id
                    .eq_ignore_ascii_case(&reader.product_id)
                && calibration.logical_unit == logical_unit
        })
        .map(|calibration| calibration.slot_kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandisk_reader_without_calibration_does_not_guess_slot_kind() {
        let reader = ReaderFingerprint {
            vendor_id: "0781".into(),
            product_id: "D003".into(),
            reader_serial: None,
        };
        assert!(is_sandisk_pro_reader(&reader));
        assert_eq!(resolve_slot(&reader, 0, &[]), None);
    }

    #[test]
    fn a_captured_calibration_maps_only_the_matching_logical_unit() {
        let reader = ReaderFingerprint {
            vendor_id: "0781".into(),
            product_id: "D003".into(),
            reader_serial: Some("reader".into()),
        };
        let records = [SlotCalibration {
            reader_vendor_id: "0781".into(),
            reader_product_id: "D003".into(),
            logical_unit: 1,
            slot_kind: SlotKind::MicroSd,
            evidence_note: "controlled microSD insertion".into(),
        }];
        assert_eq!(resolve_slot(&reader, 1, &records), Some(SlotKind::MicroSd));
        assert_eq!(resolve_slot(&reader, 0, &records), None);
    }
}
