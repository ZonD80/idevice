//! iOS Installation Proxy Service Client
//!
//! Provides functionality for interacting with the installation_proxy service on iOS devices,
//! which allows querying and managing installed applications.

use std::collections::HashMap;

use log::warn;
use plist::Dictionary;

use crate::{lockdown::LockdownClient, obf, Idevice, IdeviceError, IdeviceService};

/// Client for interacting with the iOS installation proxy service
///
/// This service provides access to information about installed applications
/// and can perform application management operations.
pub struct InstallationProxyClient {
    /// The underlying device connection with established installation_proxy service
    pub idevice: Idevice,
}

impl IdeviceService for InstallationProxyClient {
    /// Returns the installation proxy service name as registered with lockdownd
    fn service_name() -> std::borrow::Cow<'static, str> {
        obf!("com.apple.mobile.installation_proxy")
    }

    /// Establishes a connection to the installation proxy service
    ///
    /// # Arguments
    /// * `provider` - Device connection provider
    ///
    /// # Returns
    /// A connected `InstallationProxyClient` instance
    ///
    /// # Errors
    /// Returns `IdeviceError` if any step of the connection process fails
    ///
    /// # Process
    /// 1. Connects to lockdownd service
    /// 2. Starts a lockdown session
    /// 3. Requests the installation proxy service port
    /// 4. Establishes connection to the service port
    /// 5. Optionally starts TLS if required by service
    async fn connect(
        provider: &dyn crate::provider::IdeviceProvider,
    ) -> Result<Self, IdeviceError> {
        let mut lockdown = LockdownClient::connect(provider).await?;
        lockdown
            .start_session(&provider.get_pairing_file().await?)
            .await?;
        let (port, ssl) = lockdown.start_service(Self::service_name()).await?;

        let mut idevice = provider.connect(port).await?;
        if ssl {
            idevice
                .start_session(&provider.get_pairing_file().await?)
                .await?;
        }

        Ok(Self::new(idevice))
    }
}

impl InstallationProxyClient {
    /// Creates a new installation proxy client from an existing device connection
    ///
    /// # Arguments
    /// * `idevice` - Pre-established device connection
    pub fn new(idevice: Idevice) -> Self {
        Self { idevice }
    }

    /// Retrieves information about installed applications
    ///
    /// # Arguments
    /// * `application_type` - Optional filter for application type:
    ///   - "System" for system applications
    ///   - "User" for user-installed applications
    ///   - "Any" for all applications (default)
    /// * `bundle_identifiers` - Optional list of specific bundle IDs to query
    ///
    /// # Returns
    /// A HashMap mapping bundle identifiers to application information plist values
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The response is malformed
    /// - The service returns an error
    ///
    /// # Example
    /// ```rust
    /// let apps = client.get_apps(Some("User".to_string()), None).await?;
    /// for (bundle_id, info) in apps {
    ///     println!("{}: {:?}", bundle_id, info);
    /// }
    /// ```
    pub async fn get_apps(
        &mut self,
        application_type: Option<String>,
        bundle_identifiers: Option<Vec<String>>,
    ) -> Result<HashMap<String, plist::Value>, IdeviceError> {
        let application_type = application_type.unwrap_or("Any".to_string());
        let mut options = plist::Dictionary::new();
        if let Some(ids) = bundle_identifiers {
            let ids = ids
                .into_iter()
                .map(plist::Value::String)
                .collect::<Vec<plist::Value>>();
            options.insert("BundleIDs".into(), ids.into());
        }
        options.insert("ApplicationType".into(), application_type.into());

        let mut req = plist::Dictionary::new();
        req.insert("Command".into(), "Lookup".into());
        req.insert("ClientOptions".into(), plist::Value::Dictionary(options));
        self.idevice
            .send_plist(plist::Value::Dictionary(req))
            .await?;

        let mut res = self.idevice.read_plist().await?;
        match res.remove("LookupResult") {
            Some(plist::Value::Dictionary(res)) => {
                Ok(res.into_iter().collect::<HashMap<String, plist::Value>>())
            }
            _ => Err(IdeviceError::UnexpectedResponse),
        }
    }

    /// Installs an application package on the device
    ///
    /// # Arguments
    /// * `package_path` - Path to the .ipa package in the AFC jail (device's installation directory)
    /// * `options` - Optional installation options as a plist dictionary
    ///
    /// # Returns
    /// `Ok(())` on successful installation
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The installation fails
    /// - The service returns an error
    ///
    /// # Note
    /// The package_path should be relative to the AFC jail root
    pub async fn install(
        &mut self,
        package_path: impl Into<String>,
        options: Option<plist::Value>,
    ) -> Result<(), IdeviceError> {
        self.install_with_callback(package_path, options, |_| async {}, ())
            .await
    }

    /// Installs an application package on the device
    ///
    /// # Arguments
    /// * `package_path` - Path to the .ipa package in the AFC jail (device's installation directory)
    /// * `options` - Optional installation options as a plist dictionary
    /// * `callback` - Progress callback that receives (percent_complete, state)
    /// * `state` - State to pass to the callback
    ///
    /// # Returns
    /// `Ok(())` on successful installation
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The installation fails
    /// - The service returns an error
    ///
    /// # Note
    /// The package_path should be relative to the AFC jail root
    pub async fn install_with_callback<Fut, S>(
        &mut self,
        package_path: impl Into<String>,
        options: Option<plist::Value>,
        callback: impl Fn((u64, S)) -> Fut,
        state: S,
    ) -> Result<(), IdeviceError>
    where
        Fut: std::future::Future<Output = ()>,
        S: Clone,
    {
        let package_path = package_path.into();
        let options = options.unwrap_or(plist::Value::Dictionary(Dictionary::new()));

        let mut command = Dictionary::new();
        command.insert("Command".into(), "Install".into());
        command.insert("ClientOptions".into(), options);
        command.insert("PackagePath".into(), package_path.into());

        self.idevice
            .send_plist(plist::Value::Dictionary(command))
            .await?;

        self.watch_completion(callback, state).await
    }

    /// Upgrades an existing application on the device
    ///
    /// # Arguments
    /// * `package_path` - Path to the .ipa package in the AFC jail (device's installation directory)
    /// * `options` - Optional upgrade options as a plist dictionary
    ///
    /// # Returns
    /// `Ok(())` on successful upgrade
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The upgrade fails
    /// - The service returns an error
    pub async fn upgrade(
        &mut self,
        package_path: impl Into<String>,
        options: Option<plist::Value>,
    ) -> Result<(), IdeviceError> {
        self.upgrade_with_callback(package_path, options, |_| async {}, ())
            .await
    }

    /// Upgrades an existing application on the device
    ///
    /// # Arguments
    /// * `package_path` - Path to the .ipa package in the AFC jail (device's installation directory)
    /// * `options` - Optional upgrade options as a plist dictionary
    /// * `callback` - Progress callback that receives (percent_complete, state)
    /// * `state` - State to pass to the callback
    ///
    /// # Returns
    /// `Ok(())` on successful upgrade
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The upgrade fails
    /// - The service returns an error
    pub async fn upgrade_with_callback<Fut, S>(
        &mut self,
        package_path: impl Into<String>,
        options: Option<plist::Value>,
        callback: impl Fn((u64, S)) -> Fut,
        state: S,
    ) -> Result<(), IdeviceError>
    where
        Fut: std::future::Future<Output = ()>,
        S: Clone,
    {
        let package_path = package_path.into();
        let options = options.unwrap_or(plist::Value::Dictionary(Dictionary::new()));

        let mut command = Dictionary::new();
        command.insert("Command".into(), "Upgrade".into());
        command.insert("ClientOptions".into(), options);
        command.insert("PackagePath".into(), package_path.into());

        self.idevice
            .send_plist(plist::Value::Dictionary(command))
            .await?;

        self.watch_completion(callback, state).await
    }

    /// Uninstalls an application from the device
    ///
    /// # Arguments
    /// * `bundle_id` - Bundle identifier of the application to uninstall
    /// * `options` - Optional uninstall options as a plist dictionary
    ///
    /// # Returns
    /// `Ok(())` on successful uninstallation
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The uninstallation fails
    /// - The service returns an error
    pub async fn uninstall(
        &mut self,
        bundle_id: impl Into<String>,
        options: Option<plist::Value>,
    ) -> Result<(), IdeviceError> {
        self.uninstall_with_callback(bundle_id, options, |_| async {}, ())
            .await
    }

    /// Uninstalls an application from the device
    ///
    /// # Arguments
    /// * `bundle_id` - Bundle identifier of the application to uninstall
    /// * `options` - Optional uninstall options as a plist dictionary
    /// * `callback` - Progress callback that receives (percent_complete, state)
    /// * `state` - State to pass to the callback
    ///
    /// # Returns
    /// `Ok(())` on successful uninstallation
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The uninstallation fails
    /// - The service returns an error
    pub async fn uninstall_with_callback<Fut, S>(
        &mut self,
        bundle_id: impl Into<String>,
        options: Option<plist::Value>,
        callback: impl Fn((u64, S)) -> Fut,
        state: S,
    ) -> Result<(), IdeviceError>
    where
        Fut: std::future::Future<Output = ()>,
        S: Clone,
    {
        let bundle_id = bundle_id.into();
        let options = options.unwrap_or(plist::Value::Dictionary(Dictionary::new()));

        let mut command = Dictionary::new();
        command.insert("Command".into(), "Uninstall".into());
        command.insert("ApplicationIdentifier".into(), bundle_id.into());
        command.insert("ClientOptions".into(), options);

        self.idevice
            .send_plist(plist::Value::Dictionary(command))
            .await?;

        self.watch_completion(callback, state).await
    }

    /// Checks if the device capabilities match the required capabilities
    ///
    /// # Arguments
    /// * `capabilities` - List of required capabilities as plist values
    /// * `options` - Optional check options as a plist dictionary
    ///
    /// # Returns
    /// `true` if all capabilities are supported, `false` otherwise
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The service returns an error
    pub async fn check_capabilities_match(
        &mut self,
        capabilities: Vec<plist::Value>,
        options: Option<plist::Value>,
    ) -> Result<bool, IdeviceError> {
        let options = options.unwrap_or(plist::Value::Dictionary(Dictionary::new()));

        let mut command = Dictionary::new();
        command.insert("Command".into(), "CheckCapabilitiesMatch".into());
        command.insert("ClientOptions".into(), options);
        command.insert("Capabilities".into(), capabilities.into());

        self.idevice
            .send_plist(plist::Value::Dictionary(command))
            .await?;
        let mut res = self.idevice.read_plist().await?;

        if let Some(caps) = res.remove("LookupResult").and_then(|x| x.as_boolean()) {
            Ok(caps)
        } else {
            Err(IdeviceError::UnexpectedResponse)
        }
    }

    /// Browses installed applications on the device
    ///
    /// # Arguments
    /// * `options` - Optional browse options as a plist dictionary
    ///
    /// # Returns
    /// A vector of plist values representing application information
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The service returns an error
    ///
    /// # Note
    /// This method streams application information in chunks and collects them into a single vector
    pub async fn browse(
        &mut self,
        options: Option<plist::Value>,
    ) -> Result<Vec<plist::Value>, IdeviceError> {
        let options = options.unwrap_or(plist::Value::Dictionary(Dictionary::new()));

        let mut command = Dictionary::new();
        command.insert("Command".into(), "Browse".into());
        command.insert("ClientOptions".into(), options);

        self.idevice
            .send_plist(plist::Value::Dictionary(command))
            .await?;

        let mut values = Vec::new();
        loop {
            let mut res = self.idevice.read_plist().await?;

            if let Some(list) = res.remove("CurrentList").and_then(|x| x.into_array()) {
                for v in list.into_iter() {
                    values.push(v);
                }
            } else {
                warn!("browse didn't contain current list");
                break;
            }

            if let Some(status) = res.get("Status").and_then(|x| x.as_string()) {
                if status == "Complete" {
                    break;
                }
            }
        }
        Ok(values)
    }

    /// Watches for operation completion and handles progress callbacks
    ///
    /// # Arguments
    /// * `callback` - Optional progress callback that receives (percent_complete, state)
    /// * `state` - Optional state to pass to the callback
    ///
    /// # Returns
    /// `Ok(())` when the operation completes successfully
    ///
    /// # Errors
    /// Returns `IdeviceError` if:
    /// - Communication fails
    /// - The operation fails
    /// - The service returns an error
    async fn watch_completion<Fut, S>(
        &mut self,
        callback: impl Fn((u64, S)) -> Fut,
        state: S,
    ) -> Result<(), IdeviceError>
    where
        Fut: std::future::Future<Output = ()>,
        S: Clone,
    {
        loop {
            let mut res = self.idevice.read_plist().await?;

            if let Some(e) = res.remove("ErrorDescription").and_then(|x| x.into_string()) {
                return Err(IdeviceError::InstallationProxyOperationFailed(
                    e.to_string(),
                ));
            }

            // Extract status information for logging (but don't change callback signature)
            let mut status_text = "Installing".to_string();
            if let Some(current_operation) = res.get("CurrentOperation").and_then(|x| x.as_string()) {
                status_text = current_operation.to_string();
            } else if let Some(status_description) = res.get("StatusDescription").and_then(|x| x.as_string()) {
                status_text = status_description.to_string();
            } else if let Some(phase) = res.get("Phase").and_then(|x| x.as_string()) {
                status_text = phase.to_string();
            }

            if let Some(c) = res
                .remove("PercentComplete")
                .and_then(|x| x.as_unsigned_integer())
            {
                // Print status information to stdout for the tool to capture
                println!("Installing on device: {} ({:.1}%)", status_text, c as f64);
                callback((c, state.clone())).await;
            }

            if let Some(c) = res.remove("Status").and_then(|x| x.into_string()) {
                if c == "Complete" {
                    break;
                }
            }
        }
        Ok(())
    }
}
