//! A simulated scanner for developing the interface without hardware.
//!
//! Recognition results are the fictitious TD1 specimen printed in ICAO 9303
//! Part 5; images are drawn, never photographs of real documents.
use crate::Message;
use crate::backend::{Backend, ExportOutcome, ExportRequest, Inbound};
use base64::Engine as _;
use iced::Task;
use image::ImageEncoder as _;
use passport_scanner_protocol::{
    Command, Device, Event, Field, FieldKind, Image, Scan, Side, Source, Stage,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Default)]
pub struct Mock {
    connected: bool,
    queue: VecDeque<Event>,
    scans: Vec<Scan>,
    selected: Option<String>,
    open_document: Option<String>,
    profile: String,
    auto: bool,
    buzzer: bool,
    next: u32,
}

impl Mock {
    pub fn new() -> Self {
        Self { profile: "general".into(), ..Self::default() }
    }

    fn state(&self) -> Event {
        Event::State {
            device: Some(device()),
            scans: self.scans.clone(),
            selected: self.selected.clone(),
        }
    }

    fn history(&self) -> Event {
        Event::History {
            scans: self.scans.iter().map(|scan| scan.id.clone()).collect(),
            selected: self.selected.clone(),
        }
    }

    fn settings(&self) -> Event {
        Event::SettingsChanged {
            settings: BTreeMap::from([
                ("captureProfile".into(), self.profile.clone()),
                ("profileReady".into(), "true".into()),
                ("buzzer".into(), if self.buzzer { "on" } else { "off" }.into()),
            ]),
        }
    }

    fn capture(&mut self, side: Side, document: Option<String>) {
        self.next += 1;
        let document = document
            .or_else(|| self.open_document.clone())
            .unwrap_or_else(|| format!("d{}", self.next));
        let scan = specimen(format!("s{}", self.next), document.clone(), side);
        self.scans.retain(|s| !(s.document == document && s.side == side));
        self.scans.insert(0, scan.clone());
        self.selected = Some(scan.id.clone());
        self.open_document = (side == Side::Front).then_some(document);
        for (stage, source) in [
            (Stage::Settling, Some(Source::White)),
            (Stage::Capturing, Some(Source::White)),
            (Stage::Capturing, Some(Source::Infrared)),
            (Stage::Processing, None),
            (Stage::Recognising, None),
        ] {
            self.queue.push_back(Event::ScanProgress { stage, source });
        }
        self.queue.push_back(Event::ScanCompleted { scan: Box::new(scan) });
    }
}

impl Backend for Mock {
    fn connect(&mut self) -> Result<Vec<Event>, String> {
        self.connected = true;
        Ok(vec![
            self.state(),
            self.settings(),
            Event::CalibrationChanged { source: Some("Demo".into()), range: None },
            Event::AutoScanChanged { enabled: self.auto },
        ])
    }

    fn connected(&self) -> bool {
        self.connected
    }

    fn send(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::GetState => self.queue.push_back(self.state()),
            Command::CaptureSide { side, document } => self.capture(side, document),
            Command::StartScan => {
                let side = if self.open_document.is_some() { Side::Back } else { Side::Front };
                self.capture(side, None);
            }
            Command::FinishDocument => {
                self.open_document = None;
                self.selected = None;
                self.queue.push_back(self.history());
            }
            Command::SelectScan { id } => {
                self.selected = Some(id);
                self.queue.push_back(self.history());
            }
            Command::Discard { id } => {
                // Like the service: the whole document goes, both sides.
                let document = self.scans.iter().find(|s| s.id == id).map(|s| s.document.clone());
                self.scans.retain(|s| Some(&s.document) != document.as_ref());
                if self.selected.as_ref().is_some_and(|s| !self.scans.iter().any(|x| &x.id == s)) {
                    self.selected = None;
                }
                self.queue.push_back(self.history());
            }
            Command::SetAutoScan { enabled } => {
                self.auto = enabled;
                self.queue.push_back(Event::AutoScanChanged { enabled });
            }
            Command::SetSetting { key, value } => {
                match key.as_str() {
                    "captureProfile" => self.profile = value,
                    "buzzer" => self.buzzer = value == "on",
                    _ => {}
                }
                self.queue.push_back(self.settings());
            }
            Command::Calibrate | Command::ForgetCalibration => self
                .queue
                .push_back(Event::CalibrationChanged { source: Some("Demo".into()), range: None }),
            _ => {}
        }
        Ok(())
    }

    fn poll(&mut self) -> Vec<Inbound> {
        // One event per tick, so progress is visible the way a real scan's is.
        self.queue.pop_front().map(Inbound::Event).into_iter().collect()
    }

    fn export_pdf(&self, _: String, _: ExportRequest) -> Task<Message> {
        Task::done(Message::Exported(Ok(ExportOutcome::Unavailable)))
    }

    fn is_demo(&self) -> bool {
        true
    }
}

fn device() -> Device {
    Device {
        model: "Demo".into(),
        vendor_id: 0,
        product_id: 0,
        serial: None,
        sources: vec![Source::White, Source::Infrared],
    }
}

const MRZ: [&str; 3] = [
    "I<UTOD231458907<<<<<<<<<<<<<<<",
    "7408122F1204159UTO<<<<<<<<<<<6",
    "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
];

fn specimen(id: String, document: String, side: Side) -> Scan {
    let field = |kind, value: &str, check| Field { kind, value: value.into(), check, span: None };
    let back = side == Side::Back;
    Scan {
        id,
        document,
        side,
        at: String::new(),
        format: back.then(|| "TD1".into()),
        mrz: if back { MRZ.map(String::from).to_vec() } else { vec![] },
        mrz_check: back.then_some(true),
        fields: if back {
            vec![
                field(FieldKind::DocumentType, "I", None),
                field(FieldKind::IssuingState, "UTO", None),
                field(FieldKind::DocumentNumber, "D23145890", Some(true)),
                field(FieldKind::DateOfBirth, "1974-08-12", Some(true)),
                field(FieldKind::Sex, "F", None),
                field(FieldKind::DateOfExpiry, "2012-04-15", Some(true)),
                field(FieldKind::Nationality, "UTO", None),
                field(FieldKind::Surname, "ERIKSSON", None),
                field(FieldKind::GivenName, "ANNA MARIA", None),
            ]
        } else {
            vec![]
        },
        images: [Source::White, Source::Infrared]
            .into_iter()
            .map(|source| card(side, source))
            .collect(),
    }
}

/// An ID-1 shaped drawing: header band, photo box, and MRZ rows on the back.
fn card(side: Side, source: Source) -> Image {
    const W: u32 = 428;
    const H: u32 = 270;
    let infrared = source == Source::Infrared;
    let pixels = image::RgbImage::from_fn(W, H, |x, y| {
        let tone = |r: u8, g: u8, b: u8| {
            if infrared {
                let grey = ((r as u32 * 3 + g as u32 * 6 + b as u32) / 10) as u8;
                image::Rgb([grey; 3])
            } else {
                image::Rgb([r, g, b])
            }
        };
        let border = x < 6 || y < 6 || x >= W - 6 || y >= H - 6;
        let header = (16..40).contains(&y) && (16..W - 16).contains(&x);
        let photo = side == Side::Front && (300..400).contains(&x) && (60..200).contains(&y);
        let mrz_row = side == Side::Back
            && (20..W - 20).contains(&x)
            && [(190, 202), (212, 224), (234, 246)].iter().any(|(a, b)| (*a..*b).contains(&y))
            && (x / 9) % 2 == 0;
        if border {
            tone(20, 22, 26)
        } else if header {
            tone(60, 90, 160)
        } else if photo {
            tone(110, 112, 118)
        } else if mrz_row {
            tone(30, 30, 34)
        } else {
            tone(214, 228, 240)
        }
    });
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels.as_raw(), W, H, image::ExtendedColorType::Rgb8)
        .expect("in-memory PNG encoding cannot fail");
    Image {
        source,
        url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ),
        width: W,
        height: H,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(mock: &mut Mock) -> Vec<Event> {
        std::iter::from_fn(|| mock.poll().pop())
            .filter_map(|i| match i {
                Inbound::Event(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn front_then_back_forms_one_document_with_a_verified_specimen() {
        let mut mock = Mock::new();
        assert!(mock.connect().unwrap().iter().any(|e| matches!(e, Event::State { .. })));
        mock.send(Command::CaptureSide { side: Side::Front, document: None }).unwrap();
        mock.send(Command::CaptureSide { side: Side::Back, document: None }).unwrap();
        let completed: Vec<Scan> = drain(&mut mock)
            .into_iter()
            .filter_map(|e| match e {
                Event::ScanCompleted { scan } => Some(*scan),
                _ => None,
            })
            .collect();
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].document, completed[1].document);
        assert!(completed[1].checks_passed());
        assert!(completed.iter().all(|s| s.images.len() == 2));
    }

    #[test]
    fn specimen_check_digits_are_correct() {
        let digit = |s: &str| {
            s.bytes()
                .zip([7, 3, 1].iter().cycle())
                .map(|(c, w)| {
                    let v = match c {
                        b'0'..=b'9' => c - b'0',
                        b'A'..=b'Z' => c - b'A' + 10,
                        _ => 0,
                    };
                    v as u32 * w
                })
                .sum::<u32>()
                % 10
        };
        assert_eq!(digit("D23145890"), 7);
        assert_eq!(digit("740812"), 2);
        assert_eq!(digit("120415"), 9);
    }
}
