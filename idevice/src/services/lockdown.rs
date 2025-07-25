//! iOS Lockdown Service Client
//!
//! Provides functionality for interacting with the lockdown service on iOS devices,
//! which is the primary service for device management and service discovery.

use log::error;
use plist::Value;
use serde::{Deserialize, Serialize};

use crate::{obf, pairing_file, Idevice, IdeviceError, IdeviceService};

/// Client for interacting with the iOS lockdown service
///
/// This is the primary service for device management and provides:
/// - Access to device information and settings
/// - Service discovery and port allocation
/// - Session management and security
pub struct LockdownClient {
    /// The underlying device connection with established lockdown service
    pub idevice: crate::Idevice,
}

impl IdeviceService for LockdownClient {
    /// Returns the lockdown service name as registered with the device
    fn service_name() -> std::borrow::Cow<'static, str> {
        obf!("com.apple.mobile.lockdown")
    }

    /// Establishes a connection to the lockdown service
    ///
    /// # Arguments
    /// * `provider` - Device connection provider
    ///
    /// # Returns
    /// A connected `LockdownClient` instance
    ///
    /// # Errors
    /// Returns `IdeviceError` if connection fails
    async fn connect(
        provider: &dyn crate::provider::IdeviceProvider,
    ) -> Result<Self, IdeviceError> {
        let idevice = provider.connect(Self::LOCKDOWND_PORT).await?;
        Ok(Self::new(idevice))
    }
}

/// Internal structure for lockdown protocol requests
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LockdownRequest {
    label: String,
    key: Option<String>,
    request: String,
}

impl LockdownClient {
    /// The default TCP port for the lockdown service
    pub const LOCKDOWND_PORT: u16 = 62078;

    /// Creates a new lockdown client from an existing device connection
    ///
    /// # Arguments
    /// * `idevice` - Pre-established device connection
    pub fn new(idevice: Idevice) -> Self {
        Self { idevice }
    }

    /// Retrieves a specific value from the device
    ///
    /// # Arguments
    /// * `value` - The name of the value to retrieve (e.g., "DeviceName")
    ///
    /// # Returns
    /// The requested value as a plist Value
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The requested value doesn't exist
    /// - The response is malformed
    ///
    /// # Example
    /// ```rust
    /// let device_name = client.get_value("DeviceName").await?;
    /// println!("Device name: {:?}", device_name);
    /// ```
    pub async fn get_value(
        &mut self,
        key: impl Into<String>,
        domain: Option<String>,
    ) -> Result<Value, IdeviceError> {
        let key = key.into();

        let mut request = plist::Dictionary::new();
        request.insert("Label".into(), self.idevice.label.clone().into());
        request.insert("Request".into(), "GetValue".into());
        request.insert("Key".into(), key.into());

        if let Some(domain) = domain {
            request.insert("Domain".into(), domain.into());
        }

        self.idevice
            .send_plist(plist::Value::Dictionary(request))
            .await?;
        let message: plist::Dictionary = self.idevice.read_plist().await?;
        match message.get("Value") {
            Some(m) => Ok(m.to_owned()),
            None => Err(IdeviceError::UnexpectedResponse),
        }
    }

    /// Retrieves all available values from the device
    ///
    /// # Returns
    /// A dictionary containing all device values
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The response is malformed
    ///
    /// # Example
    /// ```rust
    /// let all_values = client.get_all_values().await?;
    /// for (key, value) in all_values {
    ///     println!("{}: {:?}", key, value);
    /// }
    /// ```
    pub async fn get_all_values(
        &mut self,
        domain: Option<String>,
    ) -> Result<plist::Dictionary, IdeviceError> {
        let mut request = plist::Dictionary::new();
        request.insert("Label".into(), self.idevice.label.clone().into());
        request.insert("Request".into(), "GetValue".into());
        if let Some(domain) = domain {
            request.insert("Domain".into(), domain.into());
        }

        let message = plist::to_value(&request)?;
        self.idevice.send_plist(message).await?;
        let message: plist::Dictionary = self.idevice.read_plist().await?;
        match message.get("Value") {
            Some(m) => Ok(plist::from_value(m)?),
            None => Err(IdeviceError::UnexpectedResponse),
        }
    }

    /// Sets a value on the device
    ///
    /// # Arguments
    /// * `key` - The key to set
    /// * `value` - The plist value to set
    /// * `domain` - An optional domain to set by
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The response is malformed
    ///
    /// # Example
    /// ```rust
    /// client.set_value("EnableWifiDebugging", true.into(), Some("com.apple.mobile.wireless_lockdown".to_string())).await?;
    /// ```
    pub async fn set_value(
        &mut self,
        key: impl Into<String>,
        value: Value,
        domain: Option<String>,
    ) -> Result<(), IdeviceError> {
        let key = key.into();

        let mut req = plist::Dictionary::new();
        req.insert("Label".into(), self.idevice.label.clone().into());
        req.insert("Request".into(), "SetValue".into());
        req.insert("Key".into(), key.into());
        req.insert("Value".into(), value);

        if let Some(domain) = domain {
            req.insert("Domain".into(), domain.into());
        }

        self.idevice
            .send_plist(plist::Value::Dictionary(req))
            .await?;
        self.idevice.read_plist().await?;

        Ok(())
    }

    /// Starts a secure TLS session with the device
    ///
    /// # Arguments
    /// * `pairing_file` - Contains the device's identity and certificates
    ///
    /// # Returns
    /// `Ok(())` on successful session establishment
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - No connection is established
    /// - The session request is denied
    /// - TLS handshake fails
    pub async fn start_session(
        &mut self,
        pairing_file: &pairing_file::PairingFile,
    ) -> Result<(), IdeviceError> {
        if self.idevice.socket.is_none() {
            return Err(IdeviceError::NoEstablishedConnection);
        }

        let mut request = plist::Dictionary::new();
        request.insert(
            "Label".to_string(),
            plist::Value::String(self.idevice.label.clone()),
        );

        request.insert(
            "Request".to_string(),
            plist::Value::String("StartSession".to_string()),
        );
        request.insert(
            "HostID".to_string(),
            plist::Value::String(pairing_file.host_id.clone()),
        );
        request.insert(
            "SystemBUID".to_string(),
            plist::Value::String(pairing_file.system_buid.clone()),
        );

        self.idevice
            .send_plist(plist::Value::Dictionary(request))
            .await?;

        let response = self.idevice.read_plist().await?;
        match response.get("EnableSessionSSL") {
            Some(plist::Value::Boolean(enable)) => {
                if !enable {
                    return Err(IdeviceError::UnexpectedResponse);
                }
            }
            _ => {
                return Err(IdeviceError::UnexpectedResponse);
            }
        }

        self.idevice.start_session(pairing_file).await?;
        Ok(())
    }

    /// Requests to start a service on the device
    ///
    /// # Arguments
    /// * `identifier` - The service identifier (e.g., "com.apple.debugserver")
    ///
    /// # Returns
    /// A tuple containing:
    /// - The port number where the service is available
    /// - A boolean indicating whether SSL should be used
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - The service cannot be started
    /// - The response is malformed
    /// - The requested service doesn't exist
    pub async fn start_service(
        &mut self,
        identifier: impl Into<String>,
    ) -> Result<(u16, bool), IdeviceError> {
        let identifier = identifier.into();
        let mut req = plist::Dictionary::new();
        req.insert("Request".into(), "StartService".into());
        req.insert("Service".into(), identifier.into());
        self.idevice
            .send_plist(plist::Value::Dictionary(req))
            .await?;
        let response = self.idevice.read_plist().await?;

        let ssl = match response.get("EnableServiceSSL") {
            Some(plist::Value::Boolean(ssl)) => ssl.to_owned(),
            _ => false, // over USB, this option won't exist
        };

        match response.get("Port") {
            Some(plist::Value::Integer(port)) => {
                if let Some(port) = port.as_unsigned() {
                    Ok((port as u16, ssl))
                } else {
                    error!("Port isn't an unsigned integer!");
                    Err(IdeviceError::UnexpectedResponse)
                }
            }
            _ => {
                error!("Response didn't contain an integer port");
                Err(IdeviceError::UnexpectedResponse)
            }
        }
    }

    /// Generates a pairing file and sends it to the device for trusting.
    /// Note that this does NOT save the file to usbmuxd's cache. That's a responsibility of the
    /// caller.
    /// Note that this function is computationally heavy in a debug build.
    ///
    /// # Arguments
    /// * `host_id` - The host ID, in the form of a UUID. Typically generated from the host name
    /// * `system_buid` - UUID fetched from usbmuxd. Doesn't appear to affect function.
    ///
    /// # Returns
    /// The newly generated pairing record
    ///
    /// # Errors
    /// Returns `IdeviceError`
    #[cfg(feature = "pair")]
    pub async fn pair(
        &mut self,
        host_id: impl Into<String>,
        system_buid: impl Into<String>,
    ) -> Result<crate::pairing_file::PairingFile, IdeviceError> {
        let host_id = host_id.into();
        let system_buid = system_buid.into();

        let pub_key = self.get_value("DevicePublicKey", None).await?;
        let pub_key = match pub_key.as_data().map(|x| x.to_vec()) {
            Some(p) => p,
            None => {
                log::warn!("Did not get public key data response");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        let wifi_mac = self.get_value("WiFiAddress", None).await?;
        let wifi_mac = match wifi_mac.as_string() {
            Some(w) => w,
            None => {
                log::warn!("Did not get WiFiAddress string");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        let ca = crate::ca::generate_certificates(&pub_key, None).unwrap();
        let mut pair_record = plist::Dictionary::new();
        pair_record.insert("DevicePublicKey".into(), plist::Value::Data(pub_key));
        pair_record.insert("DeviceCertificate".into(), plist::Value::Data(ca.dev_cert));
        pair_record.insert(
            "HostCertificate".into(),
            plist::Value::Data(ca.host_cert.clone()),
        );
        pair_record.insert("HostID".into(), host_id.into());
        pair_record.insert("RootCertificate".into(), plist::Value::Data(ca.host_cert));
        pair_record.insert(
            "RootPrivateKey".into(),
            plist::Value::Data(ca.private_key.clone()),
        );
        pair_record.insert("WiFiMACAddress".into(), wifi_mac.into());
        pair_record.insert("SystemBUID".into(), system_buid.into());

        let mut options = plist::Dictionary::new();
        options.insert("ExtendedPairingErrors".into(), true.into());

        let mut req = plist::Dictionary::new();
        req.insert("Label".into(), self.idevice.label.clone().into());
        req.insert("Request".into(), "Pair".into());
        req.insert(
            "PairRecord".into(),
            plist::Value::Dictionary(pair_record.clone()),
        );
        req.insert("ProtocolVersion".into(), "2".into());
        req.insert("PairingOptions".into(), plist::Value::Dictionary(options));

        loop {
            self.idevice.send_plist(req.clone().into()).await?;
            match self.idevice.read_plist().await {
                Ok(escrow) => {
                    pair_record.insert("HostPrivateKey".into(), plist::Value::Data(ca.private_key));
                    if let Some(escrow) = escrow.get("EscrowBag").and_then(|x| x.as_data()) {
                        pair_record.insert("EscrowBag".into(), plist::Value::Data(escrow.to_vec()));
                    }

                    let p = crate::pairing_file::PairingFile::from_value(
                        &plist::Value::Dictionary(pair_record),
                    )?;

                    break Ok(p);
                }
                Err(IdeviceError::PairingDialogResponsePending) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                Err(e) => break Err(e),
            }
        }
    }
}

impl From<Idevice> for LockdownClient {
    /// Converts an existing device connection into a lockdown client
    fn from(value: Idevice) -> Self {
        Self::new(value)
    }
}
