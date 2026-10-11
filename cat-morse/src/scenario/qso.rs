// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The per-kind script builders.

use std::time::Duration;

use super::callsign::{call_from, fresh_call, home_of, station_call, Callsign, Home};
use super::data::{
    Names, ANTENNAS, PORTABLE_ANTENNAS, POWERS, QRP_POWERS, RIGS, RST, SOTA_REGIONS, WX,
};
use super::world::{by_dxcc, Entity, USA, WORLD};
use super::{
    ContestExchange, Party, Persona, ScenarioConfig, ScenarioError, ScenarioKind, Script,
    SignalProfile, Start, Station, Transmission,
};
use crate::rng::Rng;
use crate::timing::Speed;

/// Chance a rag-chew partner is maritime mobile.
const P_MARITIME: f64 = 0.03;

const STREAM_ME: u64 = 1;
const STREAM_STATIONS: u64 = 2;
const STREAM_TEXT: u64 = 3;
const STREAM_TIMING: u64 = 4;

struct Builder<'a> {
    cfg: &'a ScenarioConfig,
    me: Persona,
    stations: Vec<Station>,
    tx: Vec<Transmission>,
    st_rng: Rng,
    /// An entity no further station may come from (the DX being chased).
    avoid: Option<u16>,
    text: Rng,
    timing: Rng,
}

pub(super) fn build(cfg: &ScenarioConfig, rng: &mut Rng) -> Result<Script, ScenarioError> {
    let mut me_rng = rng.fork(STREAM_ME);
    let me = my_persona(&mut me_rng, &cfg.my_call);
    let mut b = Builder {
        cfg,
        me,
        stations: Vec::new(),
        tx: Vec::new(),
        st_rng: rng.fork(STREAM_STATIONS),
        avoid: None,
        text: rng.fork(STREAM_TEXT),
        timing: rng.fork(STREAM_TIMING),
    };
    match cfg.kind {
        ScenarioKind::RagChew => b.rag_chew()?,
        ScenarioKind::Contest {
            exchange,
            cut_numbers,
        } => b.contest(exchange, cut_numbers)?,
        ScenarioKind::Pota { park_to_park } => b.pota(park_to_park, &mut me_rng)?,
        ScenarioKind::Sota { summit_to_summit } => b.sota(summit_to_summit, &mut me_rng)?,
        ScenarioKind::DxPileup { callers } => b.pileup(callers)?,
        ScenarioKind::DxChase { callers } => b.chase(callers)?,
    }
    Ok(Script {
        kind: cfg.kind,
        me: b.me,
        stations: b.stations,
        transmissions: b.tx,
    })
}

fn pick_str(rng: &mut Rng, items: &[&str]) -> String {
    (*rng.pick(items)).to_string()
}

fn persona(rng: &mut Rng, call: Callsign, home: Option<Home>, portable: bool) -> Persona {
    let (name, qth, region, zone) = match home {
        Some(home) => {
            let names = home.entity.names.pool();
            let (qth, region) = match home.region {
                // A call area: a town in one of its subdivisions; the region
                // is a state/province code an exchange sends (two letters),
                // else the entity.
                Some(r) => {
                    let (code, towns) = rng.pick(r.subdivisions);
                    let region = if code.len() == 2 {
                        (*code).to_string()
                    } else {
                        home.entity.name.to_string()
                    };
                    (pick_str(rng, towns), region)
                }
                None => (
                    pick_str(rng, home.entity.towns),
                    home.entity.name.to_string(),
                ),
            };
            (pick_str(rng, names), qth, region, home.cq())
        }
        None => (
            pick_str(rng, Names::Expedition.pool()),
            "HOME".to_string(),
            "DX".to_string(),
            rng.range_u32(1, 40) as u8,
        ),
    };
    let (power_w, antenna) = if portable {
        (*rng.pick(QRP_POWERS), pick_str(rng, PORTABLE_ANTENNAS))
    } else {
        (*rng.pick(POWERS), pick_str(rng, ANTENNAS))
    };
    Persona {
        call,
        name,
        qth,
        region,
        zone,
        rig: pick_str(rng, RIGS),
        power_w,
        antenna,
        reference: None,
    }
}

fn my_persona(rng: &mut Rng, call: &Callsign) -> Persona {
    persona(rng, call.clone(), home_of(call), false)
}

/// The exchange's location field: a US state, or `DX`.
fn exchange_region(p: &Persona) -> &str {
    if p.region.len() == 2 {
        &p.region
    } else {
        "DX"
    }
}

fn pota_ref(rng: &mut Rng, iso: &str) -> String {
    let n = if iso == "US" && rng.chance(0.1) {
        rng.range_u32(10_000, 11_999)
    } else {
        rng.range_u32(1, 9_999)
    };
    format!("{iso}-{n:04}")
}

/// A SOTA association, and the home a station activating in it has.
fn sota_home(assoc: &str, rng: &mut Rng) -> Home {
    let us = |d: u8| {
        let entity = by_dxcc(USA);
        Home {
            entity,
            region: entity.region("W", d),
        }
    };
    let dx = |dxcc: u16| Home {
        entity: by_dxcc(dxcc),
        region: None,
    };
    match assoc {
        "W0C" => us(0),
        "W7W" | "W7A" => us(7),
        "W6" => us(6),
        "W4C" => us(4),
        "G" => dx(223),
        "GM" => dx(279),
        "DL" => dx(230),
        "HB" => dx(287),
        "VE7" => dx(1),
        _ => us(rng.range_u32(0, 9) as u8),
    }
}

/// The US state a W-prefixed SOTA association is in.
fn sota_state(assoc: &str) -> Option<&'static str> {
    match assoc {
        "W0C" => Some("CO"),
        "W7W" => Some("WA"),
        "W7A" => Some("AZ"),
        "W6" => Some("CA"),
        "W4C" => Some("NC"),
        _ => None,
    }
}

fn sota_ref(rng: &mut Rng) -> (String, &'static str) {
    let (assoc, regions) = rng.pick(SOTA_REGIONS);
    let region = rng.pick(regions);
    let n = rng.range_u32(1, 199);
    (format!("{assoc}/{region}-{n:03}"), assoc)
}

/// Contest cut numbers: 0 as T, 9 as N.
fn cut(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0' => 'T',
            '9' => 'N',
            c => c,
        })
        .collect()
}

impl Builder<'_> {
    fn signal(&mut self) -> Result<SignalProfile, ScenarioError> {
        let d = &self.cfg.difficulty;
        let rng = &mut self.st_rng;
        let wpm = f64::from(rng.range_u32(d.wpm.0, d.wpm.1));
        let speed = match d.farnsworth_effective {
            Some(e) if e < wpm => Speed::farnsworth(wpm, e)?,
            _ => Speed::wpm(wpm)?,
        };
        let fist = *rng.pick(&d.fists);
        let pitch = rng.range_f64(
            f64::from(d.pitch_offset_hz.0),
            f64::from(d.pitch_offset_hz.1),
        );
        let strength = rng.range_f64(f64::from(d.strength_db.0), f64::from(d.strength_db.1));
        Ok(SignalProfile {
            pitch_offset_hz: (pitch.round()) as f32,
            speed,
            fist,
            fist_seed: rng.next_u64(),
            strength_db: ((strength * 10.0).round() / 10.0) as f32,
        })
    }

    fn taken(&self) -> Vec<Callsign> {
        self.stations
            .iter()
            .map(|s| s.persona.call.clone())
            .collect()
    }

    /// Adds a station from `home` (or a random US/DX home), returns its index.
    fn add_station(&mut self, home: Option<Home>, dx_share: f64) -> Result<usize, ScenarioError> {
        // A station drawn at random may be portable or abroad; one placed
        // for an activation keeps a plain call (the activation adds `/P`).
        let random = home.is_none();
        let home = match home {
            Some(h) => h,
            None => {
                let me = home_of(&self.cfg.my_call);
                if self.st_rng.chance(dx_share) {
                    Home::random_dx(&mut self.st_rng, me, self.cfg.difficulty.hf_band)
                } else {
                    Home::random_local(&mut self.st_rng, me)
                }
            }
        };
        self.place(home, random)
    }

    /// Adds a station at `home`, its call sometimes portable or abroad if
    /// `decorate`; returns its index.
    fn place(&mut self, mut home: Home, decorate: bool) -> Result<usize, ScenarioError> {
        let taken = self.taken();
        // A home with no call left (a one-call entity already on the air)
        // gives way to another DX home.
        let call = loop {
            let h = home;
            if let Some(c) = fresh_call(
                &mut self.st_rng,
                &mut |r| {
                    if decorate {
                        station_call(r, &h)
                    } else {
                        call_from(r, &h)
                    }
                },
                &self.cfg.my_call,
                &taken,
            ) {
                break c;
            }
            let me = home_of(&self.cfg.my_call);
            let own = me.map_or(USA, |h| h.entity.dxcc);
            let not: Vec<u16> = std::iter::once(own).chain(self.avoid).collect();
            home = Home::random_caller(&mut self.st_rng, me, &not, self.cfg.difficulty.hf_band);
        };
        let p = persona(&mut self.st_rng, call, Some(home), false);
        let signal = self.signal()?;
        self.stations.push(Station { persona: p, signal });
        Ok(self.stations.len() - 1)
    }

    fn gap(&mut self) -> Start {
        Start::AfterPrevious {
            gap: Duration::from_millis(u64::from(self.timing.range_u32(300, 1200))),
        }
    }

    fn say(&mut self, from: Party, text: String) {
        let start = self.gap();
        self.push(from, text, start);
    }

    fn push(&mut self, from: Party, text: String, start: Start) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        self.tx.push(Transmission { from, text, start });
    }

    fn repeat(&mut self) -> bool {
        self.text.chance(self.cfg.difficulty.repeat_requests)
    }

    fn call_of(&self, i: usize) -> String {
        self.stations[i].persona.call.to_string()
    }

    fn me_call(&self) -> String {
        self.me.call.to_string()
    }

    fn greeting(&mut self) -> &'static str {
        const GREETINGS: [&str; 5] = ["GM", "GA", "GE", "HI", "GD"];
        GREETINGS[self.text.range_u32(0, 4) as usize]
    }

    fn temp(&mut self, p: &Persona) -> String {
        if p.region.len() == 2 {
            format!("{}F", self.text.range_u32(10, 95))
        } else {
            format!("{}C", self.text.range_u32(0, 35))
        }
    }

    fn rag_chew(&mut self) -> Result<(), ScenarioError> {
        let them = self.add_station(None, 0.3)?;
        if self.st_rng.chance(P_MARITIME) {
            // Maritime mobile: on a ship, in no DXCC entity.
            let p = &mut self.stations[them].persona;
            p.call = Callsign::parse(p.call.base())
                .expect("a base call is a call")
                .with_suffix("MM");
            p.qth = "AT SEA".to_string();
            p.region = "MM".to_string();
        }
        let t = Party::Them(them);
        let c = self.call_of(them);
        let me = self.me_call();
        let cq = match self.text.range_u32(0, 2) {
            0 => format!("CQ CQ CQ DE {c} {c} {c} K"),
            1 => format!("CQ CQ DE {c} {c} K"),
            _ => format!("CQ CQ CQ DE {c} {c} <AR> K"),
        };
        self.say(t, cq.clone());
        if self.repeat() {
            // Nobody came back the first time.
            self.say(t, cq);
        }
        let answer = if self.text.chance(0.5) {
            format!("{c} DE {me} {me} K")
        } else {
            format!("{c} DE {me} {me} <AR>")
        };
        self.say(Party::You, answer);
        let p = self.stations[them].persona.clone();
        let rst = pick_str(&mut self.text, RST);
        let g = self.greeting();
        let qth = format!("{}, {}", p.qth, p.region);
        self.say(
            t,
            format!(
                "{me} DE {c} {g} TNX FER CALL <BT> UR RST {rst} {rst} <BT> NAME {n} {n} <BT> \
                 QTH {qth} {qth} <BT> HW CPY? <AR> {me} DE {c} <KN>",
                n = p.name
            ),
        );
        if self.repeat() {
            self.say(Party::You, format!("{c} DE {me} PSE RPT NAME ES QTH <KN>"));
            self.say(
                t,
                format!(
                    "{me} DE {c} NAME {n} {n} {n} <BT> QTH {q} {q} <KN>",
                    n = p.name,
                    q = p.qth
                ),
            );
        }
        let my = self.me.clone();
        let my_rst = pick_str(&mut self.text, RST);
        self.say(
            Party::You,
            format!(
                "{c} DE {me} R TNX {n} <BT> UR RST {my_rst} {my_rst} <BT> NAME {mn} {mn} <BT> \
                 QTH {mq} {mq} <BT> HW? <AR> {c} DE {me} <KN>",
                n = p.name,
                mn = my.name,
                mq = my.qth
            ),
        );
        let wx = pick_str(&mut self.text, WX);
        let temp = self.temp(&p);
        self.say(
            t,
            format!(
                "{me} DE {c} R FB {mn} <BT> RIG {rig} PWR {pw}W ANT {ant} <BT> WX {wx} {temp} \
                 <BT> <AR> {me} DE {c} <KN>",
                mn = my.name,
                rig = p.rig,
                pw = p.power_w,
                ant = p.antenna
            ),
        );
        if self.repeat() {
            self.say(Party::You, format!("{c} DE {me} PSE RPT RIG? <KN>"));
            self.say(t, format!("{me} DE {c} RIG {r} {r} <KN>", r = p.rig));
        }
        let my_wx = pick_str(&mut self.text, WX);
        let my_temp = self.temp(&my);
        self.say(
            Party::You,
            format!(
                "{c} DE {me} R <BT> RIG {r} PWR {pw}W ANT {a} <BT> WX {my_wx} {my_temp} <BT> \
                 TNX FER QSO {n} 73 <AR> {c} DE {me} <KN>",
                r = my.rig,
                pw = my.power_w,
                a = my.antenna,
                n = p.name
            ),
        );
        let close = *self.text.pick(&["73 ES GUD DX", "73 ES CUL", "73 GL"]);
        self.say(
            t,
            format!(
                "{me} DE {c} TNX FER FB QSO {mn} <BT> {close} <SK> {me} DE {c} TU E E",
                mn = my.name
            ),
        );
        self.say(Party::You, format!("{c} DE {me} TU 73 <SK> E E"));
        Ok(())
    }

    fn contest(
        &mut self,
        exchange: ContestExchange,
        cut_numbers: bool,
    ) -> Result<(), ScenarioError> {
        let n = self.text.range_u32(3, 5);
        let me = self.me_call();
        let mut my_serial = self.text.range_u32(1, 300);
        for _ in 0..n {
            let them = self.add_station(None, 0.4)?;
            let t = Party::Them(them);
            let c = self.call_of(them);
            let cq = match self.text.range_u32(0, 2) {
                0 => format!("CQ TEST {c} {c}"),
                1 => format!("TEST {c}"),
                _ => format!("CQ {c} TEST"),
            };
            self.say(t, cq);
            self.say(Party::You, me.clone());
            if self.repeat() {
                let partial: String = me.chars().take(2).collect();
                self.say(t, format!("{partial}?"));
                self.say(Party::You, format!("{me} {me}"));
            }
            let their_x = match exchange {
                ContestExchange::CqZone => self.stations[them].persona.zone.to_string(),
                ContestExchange::Serial => {
                    let s = self.text.range_u32(1, 1500);
                    if s < 100 && self.text.chance(0.5) {
                        format!("{s:03}")
                    } else {
                        s.to_string()
                    }
                }
            };
            let my_x = match exchange {
                ContestExchange::CqZone => self.me.zone.to_string(),
                ContestExchange::Serial => {
                    my_serial += 1;
                    my_serial.to_string()
                }
            };
            let (their_x, my_x) = if cut_numbers {
                (cut(&their_x), cut(&my_x))
            } else {
                (their_x, my_x)
            };
            self.say(t, format!("{me} 5NN {their_x}"));
            if self.repeat() {
                self.say(Party::You, "AGN?".to_string());
                self.say(t, format!("5NN {their_x} {their_x}"));
            }
            self.say(Party::You, format!("TU 5NN {my_x}"));
            if self.repeat() {
                let ask = if exchange == ContestExchange::Serial {
                    "NR?"
                } else {
                    "ZONE?"
                };
                self.say(t, ask.to_string());
                self.say(Party::You, format!("{my_x} {my_x}"));
            }
            let tu = match self.text.range_u32(0, 2) {
                0 => format!("TU {c}"),
                1 => "TU".to_string(),
                _ => "TU TEST".to_string(),
            };
            self.say(t, tu);
        }
        Ok(())
    }

    fn pota(&mut self, p2p: bool, me_rng: &mut Rng) -> Result<(), ScenarioError> {
        let (home, iso) = if self.st_rng.chance(0.75) {
            (Home::random_us(&mut self.st_rng), "US")
        } else {
            let with_parks: Vec<&'static Entity> = WORLD
                .iter()
                .filter(|e| e.iso.is_some() && e.dxcc != USA)
                .collect();
            let e = *self.st_rng.pick(&with_parks);
            (
                Home::random_in(&mut self.st_rng, e),
                e.iso.expect("filtered"),
            )
        };
        let them = self.add_station(Some(home), 0.0)?;
        let park = pota_ref(&mut self.st_rng, iso);
        {
            let p = &mut self.stations[them].persona;
            p.reference = Some(park.clone());
            p.power_w = *self.st_rng.pick(QRP_POWERS);
            p.antenna = pick_str(&mut self.st_rng, PORTABLE_ANTENNAS);
        }
        if p2p {
            let my_iso = home_of(&self.me.call)
                .and_then(|h| h.entity.iso)
                .unwrap_or("US");
            self.me.reference = Some(pota_ref(me_rng, my_iso));
        }
        let t = Party::Them(them);
        let c = self.call_of(them);
        let me = self.me_call();
        let cq = if self.text.chance(0.5) {
            format!("CQ POTA CQ POTA DE {c} {c} K")
        } else {
            format!("CQ POTA DE {c} {c} {park} K")
        };
        self.say(t, cq);
        let call_in = if p2p { format!("P2P {me}") } else { me.clone() };
        self.say(Party::You, call_in);
        if self.repeat() {
            self.say(t, "QRZ?".to_string());
            self.say(Party::You, format!("{me} {me}"));
        }
        let st = exchange_region(&self.stations[them].persona).to_string();
        let g = self.greeting();
        self.say(t, format!("{me} {g} UR 5NN 5NN {st} {st} BK"));
        let mst = exchange_region(&self.me).to_string();
        let mut reply = format!("BK TU UR 5NN 5NN {mst} {mst}");
        if let Some(r) = self.me.reference.clone() {
            reply.push_str(&format!(" P2P {r} {r}"));
        }
        reply.push_str(" 72 BK");
        self.say(Party::You, reply);
        if self.repeat() {
            self.say(t, "STATE?".to_string());
            self.say(Party::You, format!("{mst} {mst}"));
        }
        if self.text.chance(0.4) {
            self.say(Party::You, "PARK?".to_string());
            self.say(t, format!("{park} {park}"));
        }
        let close = *self.text.pick(&["TU 72 EE", "R TU 72 GL EE", "TU EE"]);
        self.say(t, close.to_string());
        Ok(())
    }

    fn sota(&mut self, s2s: bool, me_rng: &mut Rng) -> Result<(), ScenarioError> {
        let (summit, assoc) = sota_ref(&mut self.st_rng);
        let home = sota_home(assoc, &mut self.st_rng);
        let them = self.add_station(Some(home), 0.0)?;
        if let (Some(area), Some(state)) = (home.region, sota_state(assoc)) {
            // An activator in the association's own state.
            if let Some((_, towns)) = area.subdivisions.iter().find(|(s, _)| *s == state) {
                let town = pick_str(&mut self.st_rng, towns);
                let p = &mut self.stations[them].persona;
                p.region = state.to_string();
                p.qth = town;
            }
        }
        {
            let portable = self.st_rng.chance(0.5);
            let p = &mut self.stations[them].persona;
            if portable {
                p.call = p.call.with_suffix("P");
            }
            p.reference = Some(summit.clone());
            p.power_w = *self.st_rng.pick(QRP_POWERS);
            p.antenna = pick_str(&mut self.st_rng, PORTABLE_ANTENNAS);
        }
        if s2s {
            self.me.reference = Some(sota_ref(me_rng).0);
        }
        let t = Party::Them(them);
        let c = self.call_of(them);
        let me = self.me_call();
        self.say(t, format!("CQ SOTA CQ SOTA DE {c} {c} K"));
        let call_in = if s2s { format!("S2S {me}") } else { me.clone() };
        self.say(Party::You, call_in);
        if self.repeat() {
            self.say(t, "QRZ?".to_string());
            self.say(Party::You, format!("{me} {me}"));
        }
        let st = exchange_region(&self.stations[them].persona).to_string();
        let g = self.greeting();
        let ref_part = if self.text.chance(0.5) {
            format!("<BT> REF {summit} {summit}")
        } else {
            String::new()
        };
        self.say(t, format!("{me} {g} UR 5NN 5NN {st} {st} {ref_part} BK"));
        let mst = exchange_region(&self.me).to_string();
        let mut reply = format!("BK R TU UR 5NN {mst} {mst}");
        if let Some(r) = self.me.reference.clone() {
            reply.push_str(&format!(" S2S {r} {r}"));
        }
        reply.push_str(" 73 BK");
        self.say(Party::You, reply);
        if ref_part.is_empty() || self.repeat() {
            self.say(Party::You, "REF?".to_string());
            self.say(t, format!("{summit} {summit}"));
        }
        let close = *self.text.pick(&["TU 73 EE", "R TU 73 EE", "TU SOTA EE"]);
        self.say(t, close.to_string());
        Ok(())
    }

    /// The part of `base` that came through a pileup: `DL1?` or `?ABC`.
    fn partial(&mut self, base: &str) -> String {
        if self.text.chance(0.5) {
            let digit = base.find(|ch: char| ch.is_ascii_digit()).unwrap_or(1);
            format!("{}?", &base[..=digit])
        } else {
            format!("?{}", &base[base.len().saturating_sub(3)..])
        }
    }

    /// Everyone in `waiting` calls at once, staggered; `you` joins in last.
    fn all_call(&mut self, waiting: &[usize], you: Option<&str>) {
        for (k, &i) in waiting.iter().enumerate() {
            let c = self.call_of(i);
            let text = if self.text.chance(0.4) {
                format!("{c} {c}")
            } else {
                c
            };
            let start = if k == 0 {
                self.gap()
            } else {
                Start::WithPrevious {
                    delay: Duration::from_millis(u64::from(self.timing.range_u32(0, 700))),
                }
            };
            self.push(Party::Them(i), text, start);
        }
        if let Some(me) = you {
            self.say(Party::You, me.to_string());
        }
    }

    fn pileup(&mut self, callers: u8) -> Result<(), ScenarioError> {
        let me = self.me_call();
        let my_zone = self.me.zone.to_string();
        // You are the DX: the callers are the world, never your own entity.
        let my_home = home_of(&self.cfg.my_call);
        let not = [my_home.map_or(0, |h| h.entity.dxcc)];
        let mut waiting: Vec<usize> = Vec::new();
        for _ in 0..callers {
            let h =
                Home::random_caller(&mut self.st_rng, my_home, &not, self.cfg.difficulty.hf_band);
            waiting.push(self.place(h, true)?);
        }
        self.say(Party::You, format!("CQ CQ DE {me} {me} K"));
        while !waiting.is_empty() {
            // Everyone still waiting calls at once, staggered.
            self.all_call(&waiting.clone(), None);
            let pick = self.text.range_u32(0, (waiting.len() - 1) as u32) as usize;
            let them = waiting.remove(pick);
            let t = Party::Them(them);
            let c = self.call_of(them);
            if self.repeat() || waiting.len() >= 4 {
                // Only part of it came through.
                let base = self.stations[them].persona.call.base().to_string();
                let partial = self.partial(&base);
                self.say(Party::You, partial);
                self.say(t, format!("{c} {c}"));
            }
            self.say(Party::You, format!("{c} 5NN {my_zone}"));
            let zone = self.stations[them].persona.zone;
            let reply = if self.text.chance(0.5) {
                format!("TU 5NN {zone}")
            } else {
                format!("R 5NN {zone} TU")
            };
            self.say(t, reply);
            if self.repeat() {
                self.say(Party::You, "AGN?".to_string());
                self.say(t, format!("5NN {zone} {zone}"));
            }
            let next = if waiting.is_empty() {
                format!("TU {me} QRT")
            } else {
                format!("TU {me} QRZ?")
            };
            self.say(Party::You, next);
        }
        Ok(())
    }

    /// You chase a rare DX: it calls CQ split, works a few of the other
    /// callers (everyone, you included, calling over one another), then
    /// comes back to you.
    fn chase(&mut self, callers: u8) -> Result<(), ScenarioError> {
        let me = self.me_call();
        let my_home = home_of(&self.cfg.my_call);
        let dx_home = Home::random_rare(&mut self.st_rng, my_home);
        let dx = self.place(dx_home, false)?;
        // From here on no station, even a fallback, is the DX's countryman.
        self.avoid = Some(dx_home.entity.dxcc);
        let mut waiting: Vec<usize> = Vec::new();
        for _ in 0..callers {
            let h = Home::random_caller(
                &mut self.st_rng,
                my_home,
                &[dx_home.entity.dxcc],
                self.cfg.difficulty.hf_band,
            );
            waiting.push(self.place(h, true)?);
        }
        let d = Party::Them(dx);
        let dc = self.call_of(dx);
        let up = self.text.range_u32(1, 5);
        let cq = match self.text.range_u32(0, 2) {
            0 => format!("CQ {dc} UP {up}"),
            1 => format!("CQ DE {dc} {dc} UP"),
            _ => format!("{dc} UP {up}"),
        };
        self.say(d, cq);
        // You get through after the DX has worked this many others.
        let before = self.text.range_u32(1, u32::from(callers).min(3)) as usize;
        let mut worked = 0;
        loop {
            self.all_call(&waiting.clone(), Some(&me));
            if worked == before || waiting.is_empty() {
                if self.repeat() {
                    let base = self.cfg.my_call.base().to_string();
                    let partial = self.partial(&base);
                    self.say(d, partial);
                    self.say(Party::You, format!("{me} {me}"));
                }
                self.say(d, format!("{me} 5NN"));
                let tu = *self.text.pick(&["TU 5NN", "5NN TU", "R 5NN TU"]);
                self.say(Party::You, tu.to_string());
                self.say(d, format!("TU {dc} UP"));
                return Ok(());
            }
            let pick = self.text.range_u32(0, (waiting.len() - 1) as u32) as usize;
            let them = waiting.remove(pick);
            let c = self.call_of(them);
            self.say(d, format!("{c} 5NN"));
            let tu = *self.text.pick(&["TU 5NN", "5NN TU", "R 5NN"]);
            self.say(Party::Them(them), tu.to_string());
            let next = *self.text.pick(&["TU UP", "TU", "UP"]);
            self.say(d, next.to_string());
            worked += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_numbers() {
        assert_eq!(cut("009"), "TTN");
        assert_eq!(cut("14"), "14");
    }

    #[test]
    fn pota_refs_are_iso_form() {
        let mut r = Rng::new(0);
        for _ in 0..500 {
            let s = pota_ref(&mut r, "US");
            assert!(s.starts_with("US-"));
            let digits = &s[3..];
            assert!((4..=5).contains(&digits.len()), "{s}");
        }
    }

    #[test]
    fn tables_are_encodable() {
        use crate::codec::tokenize;
        let mut all: Vec<&str> = Vec::new();
        for n in Names::ALL {
            all.extend(n.pool());
        }
        all.extend(RIGS);
        all.extend(ANTENNAS);
        all.extend(PORTABLE_ANTENNAS);
        all.extend(WX);
        all.extend(RST);
        for s in all {
            assert!(tokenize(s).is_ok(), "{s}");
        }
    }
}
