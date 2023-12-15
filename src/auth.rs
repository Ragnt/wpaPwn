use std::{collections::HashMap, fmt};

use libwifi::frame::{components::MacAddress, EapolKey, MessageType};

// PMKID struct definition
#[derive(Debug, Clone, Copy)]
pub struct Pmkid {
    pub id: u8,
    pub len: u8,
    pub oui: [u8; 3],
    pub oui_type: u8,
    pub pmkid: [u8; 16],
}

// PMKID struct conversion implementation
impl Pmkid {
    fn from_bytes(bytes: &[u8]) -> Self {
        // Ensure the slice has the correct length
        if bytes.len() != 22 {
            panic!("Invalid PMKID data length");
        }
        let mut pmkid = Pmkid {
            id: bytes[0],
            len: bytes[1],
            oui: [bytes[2], bytes[3], bytes[4]],
            oui_type: bytes[5],
            pmkid: [0; 16],
        };
        pmkid.pmkid.copy_from_slice(&bytes[6..]);
        pmkid
    }
}

#[derive(Clone, Debug, Default)]
pub struct FourWayHandshake {
    pub msg1: Option<EapolKey>,
    pub msg2: Option<EapolKey>,
    pub msg3: Option<EapolKey>,
    pub msg4: Option<EapolKey>,
    pub last_msg: Option<EapolKey>,
    pub eapol_client: Option<Vec<u8>>,
    pub mic: Option<[u8; 16]>,
    pub anonce: Option<[u8; 32]>,
    pub snonce: Option<[u8; 32]>,
    pub apless: bool,
    pub nc: bool,
    pub l_endian: bool,
    pub b_endian: bool,
    pub pmkid: Option<Pmkid>,
    pub mac_ap: Option<MacAddress>,
    pub mac_client: Option<MacAddress>,
    pub essid: Option<String>,
}

// Example implementation for displaying a FourWayHandshake
impl fmt::Display for FourWayHandshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Example handshake detail (customize as needed)

        write!(
            f,
            " {:<2} {:<2} {:<2} {:<2} {:<2}     {:^2}    {:^8}",
            if self.msg1.is_some() {
                "\u{2705}\0" // The check-mark is two char's wide, so we add a null char so the fmt lib doesn't add a space when padding to two.
            } else {
                "--"
            },
            if self.msg2.is_some() {
                "\u{2705}\0"
            } else {
                "--"
            },
            if self.msg3.is_some() {
                "\u{2705}\0"
            } else {
                "--"
            },
            if self.msg4.is_some() {
                "\u{2705}\0"
            } else {
                "--"
            },
            if self.mic.is_some() {
                "\u{2705}\0"
            } else {
                "--"
            },
            if self.has_pmkid() { "\u{2705}\0" } else { "--" },
            if self.complete() { "\u{2705}\0" } else { "--" },
        )
    }
}

impl FourWayHandshake {
    pub fn new() -> Self {
        FourWayHandshake {
            msg1: None,
            msg2: None,
            msg3: None,
            msg4: None,
            last_msg: None,
            eapol_client: None,
            mic: None,
            anonce: None,
            snonce: None,
            apless: false,
            nc: false,
            l_endian: false,
            b_endian: false,
            pmkid: None,
            mac_ap: None,
            mac_client: None,
            essid: None,
        }
    }

    pub fn complete(&self) -> bool {
        self.eapol_client.is_some()
            && self.mic.is_some()
            && self.anonce.is_some()
            && self.snonce.is_some()
            && self.mac_ap.is_some()
            && self.mac_client.is_some()
            && self.essid.is_some()
    }

    pub fn has_m1(&self) -> bool {
        self.msg1.is_some()
    }

    pub fn has_pmkid(&self) -> bool {
        self.pmkid.is_some()
    }

    pub fn essid_to_string(&self) -> String {
        if let Some(essid) = self.essid.clone() {
            essid
        } else {
            "".to_string()
        }
    }

    pub fn add_key(&mut self, new_key: &EapolKey) -> Result<(), &'static str> {
        let key_type = new_key.determine_key_type();
        // Define the RSN Suite OUI for PMKID validation
        let rsnsuiteoui: [u8; 3] = [0x00, 0x0f, 0xac];

        if key_type == MessageType::GTK {
            return Err("EAPOL is a GTK Update... ignoring.");
        }

        if key_type == MessageType::Message1 && self.msg1.is_none() {
            // Validate Message 1: should have no MIC, contains ANonce
            if new_key.key_mic != [0u8; 16] {
                return Err("Invalid Message 1: MIC should not be present");
            }

            // Check for PMKID presence and validity
            if new_key.key_data_length as usize == 22 {
                // Extract PMKID from the key data
                let pmkid = Pmkid::from_bytes(&new_key.key_data);

                if pmkid.oui == rsnsuiteoui
                    && pmkid.len == 0x14
                    && pmkid.oui_type == 4
                    && pmkid.pmkid.iter().any(|&x| x != 0)
                {
                    self.pmkid = Some(pmkid)
                }
            }

            self.anonce = Some(new_key.key_nonce);
            self.msg1 = Some(new_key.clone());
            self.last_msg = Some(new_key.clone());
        } else if key_type == MessageType::Message2 && self.msg2.is_none() {
            // Validate Message 2: should have MIC
            if new_key.key_mic == [0u8; 16] {
                return Err("Invalid Message 2: MIC should be present");
            }

            // Should have Snonce
            if new_key.key_nonce == [0u8; 32] {
                return Err("Invalid Message 2: Snonce should be present.");
            }

            // Compare RC to MSG 1
            if self.msg1.is_some()
                && new_key.replay_counter <= self.msg1.clone().unwrap().replay_counter
                && new_key.replay_counter > self.msg1.clone().unwrap().replay_counter + 3
            {
                return Err("Invalid Message 2: RC value not within range.");
            }

            //Temporal Checking
            if self.msg1.clone().is_some_and(|msg1| {
                new_key
                    .timestamp
                    .duration_since(msg1.timestamp)
                    .unwrap()
                    .as_secs()
                    > 2
            }) {
                return Err("Invalid Message 2: Time difference too great.");
            }

            self.snonce = Some(new_key.key_nonce);
            self.msg2 = Some(new_key.clone());
            self.last_msg = Some(new_key.clone());
            self.eapol_client = Some(new_key.to_bytes().unwrap());
            self.mic = Some(new_key.key_mic);
            // This is good news, we have collected a M2 which gives us a solid MIC, EapolClient, and SNONCE.
        } else if key_type == MessageType::Message3 && self.msg3.is_none() {
            // Validate Message 3: should have MIC, contains ANonce, GTK
            if new_key.key_mic == [0u8; 16] {
                return Err("Invalid Message 3: MIC should be present");
            }
            if new_key.key_nonce == [0u8; 32] {
                return Err("Invalid Message 3: Anonce should be present.");
            }

            // Nonce-correction logic
            self.nc = if let Some(anonce) = self.anonce {
                if new_key.key_nonce[..28] == anonce[..28] {
                    // Compare first 28 bytes
                    if new_key.key_nonce[28..] != anonce[28..] {
                        // Compare last 4 bytes
                        if anonce[31] != new_key.key_nonce[31] {
                            // Compare Byte 31 for LE
                            self.l_endian = true;
                        } else if anonce[28] != new_key.key_nonce[28] {
                            // Compare Byte 28 for BE
                            self.b_endian = true;
                        }
                        true // 0-28 are same, last 4 are different.
                    } else {
                        false // 0-28 and last four are same- no NC needed
                    }
                } else {
                    // 0-28 aren't even close, let's ditch this key.
                    return Err("Invalid Message 3: Anonce not close enough to Message 1 Anonce.");
                }
            } else {
                // We don't have an M1 to compare to, so assume it's good... and need to set the anonce.
                self.anonce = Some(new_key.key_nonce);
                false
            };

            if self.msg2.is_some()
                && new_key.replay_counter <= self.msg2.clone().unwrap().replay_counter
                && new_key.replay_counter > self.msg2.clone().unwrap().replay_counter + 3
            {
                return Err("Invalid Message 3: RC value not within range.");
            }

            //Temporal Checking
            if self.msg2.clone().is_some_and(|msg2| {
                new_key
                    .timestamp
                    .duration_since(msg2.timestamp)
                    .unwrap()
                    .as_secs()
                    > 2
            }) {
                return Err("Invalid Message 3: Time difference too great.");
            }

            self.msg3 = Some(new_key.clone());
            self.last_msg = Some(new_key.clone());
            // Message 3 cannot be used for the EAPOL_CLIENT because it is sent by the AP.
        } else if key_type == MessageType::Message4 && self.msg4.is_none() {
            // Validate Message 4: should have MIC
            if new_key.key_mic == [0u8; 16] {
                return Err("Invalid Message 4: MIC should be present");
            }
            if self.msg3.is_some()
                && new_key.replay_counter <= self.msg3.clone().unwrap().replay_counter
                && new_key.replay_counter > self.msg3.clone().unwrap().replay_counter + 3
            {
                return Err("Invalid Message 4: RC value not within range.");
            }

            //Temporal Checking
            if self.msg3.clone().is_some_and(|msg3| {
                new_key
                    .timestamp
                    .duration_since(msg3.timestamp)
                    .unwrap()
                    .as_secs()
                    > 2
            }) {
                return Err("Invalid Message 4: Time difference too great.");
            }

            self.msg4 = Some(new_key.clone());
            self.last_msg = Some(new_key.clone());
            // If we dont have an snonce, theres a chance our M4 isn't zeroed and therefore we can use the snonce from it.
            if self.snonce.is_none() && new_key.key_nonce != [0u8; 32] {
                self.snonce = Some(new_key.key_nonce);

                // If we don't have a message 2, we will use the M4 as our EAPOL_CLIENT (only if it's non-zeroed).
                if self.eapol_client.is_none() {
                    self.mic = Some(new_key.key_mic);
                    self.eapol_client = Some(new_key.to_bytes().unwrap())
                }
            }
        } else {
            return Err("Handshake already complete or message already present.");
        }
        Ok(())
    }

    pub fn to_hashcat_22000_format(&self) -> Option<String> {
        let mut output = String::new();

        if let Some(pmkid) = &self.pmkid {
            if let Some(pmkid_format) = self.generate_pmkid_hashcat_format(pmkid) {
                output += &pmkid_format;
            }
        }

        if !self.complete() && output.is_empty() {
            return None;
        } else if !self.complete() && !output.is_empty() {
            return Some(output);
        }

        output.push('\n');

        let mic_hex = self
            .mic
            .as_ref()?
            .iter()
            .fold(String::new(), |mut acc, &byte| {
                acc.push_str(&format!("{:02x}", byte));
                acc
            });

        let mac_ap_hex = self.mac_ap.as_ref()?.to_string();
        let mac_client_hex = self.mac_client.as_ref()?.to_string();

        let essid_hex =
            self.essid
                .as_ref()?
                .as_bytes()
                .iter()
                .fold(String::new(), |mut acc, &byte| {
                    acc.push_str(&format!("{:02x}", byte));
                    acc
                });

        let anonce_hex = self
            .anonce
            .as_ref()?
            .iter()
            .fold(String::new(), |mut acc, &byte| {
                acc.push_str(&format!("{:02x}", byte));
                acc
            });

        let eapol_client_hex =
            self.eapol_client
                .as_ref()?
                .iter()
                .fold(String::new(), |mut acc, &byte| {
                    acc.push_str(&format!("{:02x}", byte));
                    acc
                });

        let message_pair = self.calculate_message_pair();

        output += &format!(
            "WPA*02*{}*{}*{}*{}*{}*{}*{}",
            mic_hex,
            mac_ap_hex,
            mac_client_hex,
            essid_hex,
            anonce_hex,
            eapol_client_hex,
            message_pair
        );

        Some(output)
    }

    fn generate_pmkid_hashcat_format(&self, pmkid: &Pmkid) -> Option<String> {
        let pmkid_hex = pmkid.pmkid.iter().fold(String::new(), |mut acc, &byte| {
            acc.push_str(&format!("{:02x}", byte));
            acc
        });

        let mac_ap_hex = self.mac_ap.as_ref()?.to_string();
        let mac_client_hex = self.mac_client.as_ref()?.to_string();
        let essid_hex =
            self.essid
                .as_ref()?
                .as_bytes()
                .iter()
                .fold(String::new(), |mut acc, &byte| {
                    acc.push_str(&format!("{:02x}", byte));
                    acc
                });

        // Calculate the message pair value
        let message_pair = self.calculate_message_pair();

        Some(format!(
            "WPA*01*{}*{}*{}*{}***{}",
            pmkid_hex, mac_ap_hex, mac_client_hex, essid_hex, message_pair
        ))
    }

    fn calculate_message_pair(&self) -> String {
        let mut message_pair = 0;

        if self.apless {
            message_pair |= 0x10; // Set the AP-less bit
        }
        if self.nc {
            message_pair |= 0x80; // Set the Nonce-Correction bit
        }
        if self.l_endian {
            message_pair |= 0x20; // Set the Little Endian bit
        }
        if self.b_endian {
            message_pair |= 0x40; // Set the Big Endian bit
        }

        // Determine the basic message pair based on messages present
        if self.msg2.is_some() && self.msg3.is_some() {
            message_pair |= 0x02; // M2+M3, EAPOL from M2
        } else if self.msg1.is_some() && self.msg2.is_some() {
            message_pair |= 0x00; // M1+M2, EAPOL from M2 (challenge)
        } else if self.msg1.is_some() && self.msg4.is_some() {
            message_pair |= 0x01; // M1+M4, EAPOL from M4
        } else if self.msg3.is_some() && self.msg4.is_some() {
            message_pair |= 0x05; // M3+M4, EAPOL from M4
        }

        format!("{:02x}", message_pair)
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct HandshakeSessionKey {
    pub ap_mac: MacAddress,
    pub client_mac: MacAddress,
}

impl HandshakeSessionKey {
    fn new(ap_mac: MacAddress, client_mac: MacAddress) -> Self {
        HandshakeSessionKey { ap_mac, client_mac }
    }
}

// Stores collected 4-way-handshakes
pub struct HandshakeStorage {
    handshakes: HashMap<HandshakeSessionKey, Vec<FourWayHandshake>>,
}

impl HandshakeStorage {
    pub fn new() -> Self {
        HandshakeStorage {
            handshakes: HashMap::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.handshakes.values().map(|v| v.len()).sum()
    }

    pub fn get_handshakes(&self) -> HashMap<HandshakeSessionKey, Vec<FourWayHandshake>> {
        self.handshakes.clone()
    }

    pub fn find_handshakes_by_ap(
        &self,
        ap_mac: &MacAddress,
    ) -> HashMap<MacAddress, Vec<FourWayHandshake>> {
        self.handshakes
            .iter()
            .filter(|(key, _)| &key.ap_mac == ap_mac)
            .map(|(key, handshakes)| (key.client_mac, handshakes.clone()))
            .collect()
    }

    pub fn has_complete_handshake_for_ap(&self, ap_mac: &MacAddress) -> bool {
        self.handshakes.iter().any(|(key, handshakes)| {
            &key.ap_mac == ap_mac && handshakes.iter().any(|hs| hs.complete())
        })
    }

    pub fn has_m1_for_ap(&self, ap_mac: &MacAddress) -> bool {
        self.handshakes.iter().any(|(key, handshakes)| {
            &key.ap_mac == ap_mac && handshakes.iter().any(|hs| hs.has_m1())
        })
    }

    pub fn add_or_update_handshake(
        &mut self,
        ap_mac: &MacAddress,
        client_mac: &MacAddress,
        new_key: EapolKey,
        essid: Option<String>,
    ) -> Result<FourWayHandshake, &'static str> {
        let session_key = HandshakeSessionKey::new(*ap_mac, *client_mac);

        let handshake_list = self.handshakes.entry(session_key).or_default();
        for handshake in &mut *handshake_list {
            if handshake.add_key(&new_key).is_ok() {
                handshake.mac_ap = Some(*ap_mac);
                handshake.mac_client = Some(*client_mac);
                handshake.essid = essid;
                return Ok(handshake.clone());
            }
        }
        let mut new_handshake = FourWayHandshake::new(); // Create a new FourWayHandshake instance
        new_handshake.add_key(&new_key)?;
        new_handshake.mac_ap = Some(*ap_mac);
        new_handshake.mac_client = Some(*client_mac);
        new_handshake.essid = essid;
        let hs = new_handshake.clone();
        handshake_list.push(new_handshake.clone());
        Ok(hs)
    }
}
