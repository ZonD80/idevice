// Jackson Coxson

use log::warn;
use serde::Deserialize;

use crate::{obf, IdeviceError, ReadWrite, RsdService};

use super::CoreDeviceServiceClient;

impl<R: ReadWrite> RsdService for AppServiceClient<R> {
    fn rsd_service_name() -> std::borrow::Cow<'static, str> {
        obf!("com.apple.coredevice.appservice")
    }

    async fn from_stream(stream: R) -> Result<Self, IdeviceError> {
        Ok(Self {
            inner: CoreDeviceServiceClient::new(stream).await?,
        })
    }

    type Stream = R;
}

pub struct AppServiceClient<R: ReadWrite> {
    inner: CoreDeviceServiceClient<R>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AppListEntry {
    #[serde(rename = "isRemovable")]
    pub is_removable: bool,
    pub name: String,
    #[serde(rename = "isFirstParty")]
    pub is_first_party: bool,
    pub path: String,
    #[serde(rename = "bundleIdentifier")]
    pub bundle_identifier: String,
    #[serde(rename = "isDeveloperApp")]
    pub is_developer_app: bool,
    #[serde(rename = "bundleVersion")]
    pub bundle_version: Option<String>,
    #[serde(rename = "isInternal")]
    pub is_internal: bool,
    #[serde(rename = "isHidden")]
    pub is_hidden: bool,
    #[serde(rename = "isAppClip")]
    pub is_app_clip: bool,
    pub version: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LaunchResponse {
    #[serde(rename = "processIdentifierVersion")]
    pub process_identifier_version: u32,
    #[serde(rename = "processIdentifier")]
    pub pid: u32,
    #[serde(rename = "executableURL")]
    pub executable_url: ExecutableUrl,
    #[serde(rename = "auditToken")]
    pub audit_token: Vec<u32>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ExecutableUrl {
    pub relative: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ProcessToken {
    #[serde(rename = "processIdentifier")]
    pub pid: u32,
    #[serde(rename = "executableURL")]
    pub executable_url: Option<ExecutableUrl>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SignalResponse {
    pub process: ProcessToken,
    #[serde(rename = "deviceTimestamp")]
    pub device_timestamp: plist::Date,
    pub signal: u32,
}

/// Icon data is in a proprietary format.
///
/// ```
/// 0000: 06 00 00 00 40 06 00 00 00 00 00 00 01 00 00 00 - header
/// 0010: 00 00 a0 41 00 00 a0 41 00 00 00 00 00 00 00 00 - width x height as float
/// 0020: 00 00 a0 41 00 00 a0 41 00 00 00 00 00 00 00 00 - wdith x height (again?)
/// 0030: 00 00 00 00 03 08 08 09 2a 68 6f 7d 44 a9 b7 d0 - start of image data
/// <snip>
/// ```
///
/// The data can be parsed like so in Python
///
/// ```python
/// from PIL import Image
///
/// width, height = 20, 20 (from the float sizes)
/// with open("icon.raw", "rb") as f:
///     f.seek(0x30)
///     raw = f.read(width * height * 4)
///
/// img = Image.frombytes("RGBA", (width, height), raw)
/// img.save("icon.png")
/// ```
#[derive(Deserialize, Clone, Debug)]
pub struct IconData {
    pub data: plist::Data,
    #[serde(rename = "iconSize.height")]
    pub icon_height: f64,
    #[serde(rename = "iconSize.width")]
    pub icon_width: f64,
    #[serde(rename = "minimumSize.height")]
    pub minimum_height: f64,
    #[serde(rename = "minimumSize.width")]
    pub minimum_width: f64,
    #[serde(rename = "$classes")]
    pub classes: Vec<String>,
    #[serde(rename = "validationToken")]
    pub validation_token: plist::Data,
    pub uuid: IconUuid,
}

#[derive(Deserialize, Clone, Debug)]
pub struct IconUuid {
    #[serde(rename = "NS.uuidbytes")]
    pub bytes: plist::Data,
    #[serde(rename = "$classes")]
    pub classes: Vec<String>,
}

impl<'a, R: ReadWrite + 'a> AppServiceClient<R> {
    pub async fn new(stream: R) -> Result<Self, IdeviceError> {
        Ok(Self {
            inner: CoreDeviceServiceClient::new(stream).await?,
        })
    }

    pub fn box_inner(self) -> AppServiceClient<Box<dyn ReadWrite + 'a>> {
        AppServiceClient {
            inner: self.inner.box_inner(),
        }
    }

    pub async fn list_apps(
        &mut self,
        app_clips: bool,
        removable_apps: bool,
        hidden_apps: bool,
        internal_apps: bool,
        default_apps: bool,
    ) -> Result<Vec<AppListEntry>, IdeviceError> {
        let mut options = plist::Dictionary::new();
        options.insert("includeAppClips".into(), app_clips.into());
        options.insert("includeRemovableApps".into(), removable_apps.into());
        options.insert("includeHiddenApps".into(), hidden_apps.into());
        options.insert("includeInternalApps".into(), internal_apps.into());
        options.insert("includeDefaultApps".into(), default_apps.into());
        let res = self
            .inner
            .invoke("com.apple.coredevice.feature.listapps", Some(options))
            .await?;

        let res = match res.as_array() {
            Some(a) => a,
            None => {
                warn!("CoreDevice result was not an array");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        let mut desd = Vec::new();
        for r in res {
            let r: AppListEntry = match plist::from_value(r) {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to parse app entry: {e:?}");
                    return Err(IdeviceError::UnexpectedResponse);
                }
            };
            desd.push(r);
        }

        Ok(desd)
    }

    pub async fn launch_application(
        &mut self,
        bundle_id: impl Into<String>,
        arguments: &[&str],
        kill_existing: bool,
        start_suspended: bool,
        environment: Option<plist::Dictionary>,
        platform_options: Option<plist::Dictionary>,
    ) -> Result<LaunchResponse, IdeviceError> {
        let bundle_id = bundle_id.into();

        let req = crate::plist!({
            "applicationSpecifier": {
                "bundleIdentifier": {
                    "_0": bundle_id
                }
            },
            "options": {
                "arguments": arguments, // Now this will work directly
                "environmentVariables": environment.unwrap_or_default(),
                "standardIOUsesPseudoterminals": true,
                "startStopped": start_suspended,
                "terminateExisting": kill_existing,
                "user": {
                    "shortName": "mobile"
                },
                "platformSpecificOptions": plist::Value::Data(crate::util::plist_to_xml_bytes(&platform_options.unwrap_or_default())),
            },
            "standardIOIdentifiers": {}
        })
        .into_dictionary()
        .unwrap();

        let res = self
            .inner
            .invoke("com.apple.coredevice.feature.launchapplication", Some(req))
            .await?;

        let res = match res
            .as_dictionary()
            .and_then(|r| r.get("processToken"))
            .and_then(|x| plist::from_value(x).ok())
        {
            Some(r) => r,
            None => {
                warn!("CoreDevice res did not contain parsable processToken");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        Ok(res)
    }

    pub async fn list_processes(&mut self) -> Result<Vec<ProcessToken>, IdeviceError> {
        let res = self
            .inner
            .invoke("com.apple.coredevice.feature.listprocesses", None)
            .await?;

        let res = match res
            .as_dictionary()
            .and_then(|x| x.get("processTokens"))
            .and_then(|x| plist::from_value(x).ok())
        {
            Some(r) => r,
            None => {
                warn!("CoreDevice res did not contain parsable processToken");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        Ok(res)
    }

    /// Gives no response on failure or success
    pub async fn uninstall_app(
        &mut self,
        bundle_id: impl Into<String>,
    ) -> Result<(), IdeviceError> {
        let bundle_id = bundle_id.into();
        self.inner
            .invoke(
                "com.apple.coredevice.feature.uninstallapp",
                Some(
                    crate::plist!({"bundleIdentifier": bundle_id})
                        .into_dictionary()
                        .unwrap(),
                ),
            )
            .await?;

        Ok(())
    }

    pub async fn send_signal(
        &mut self,
        pid: u32,
        signal: u32,
    ) -> Result<SignalResponse, IdeviceError> {
        let res = self
            .inner
            .invoke(
                "com.apple.coredevice.feature.sendsignaltoprocess",
                Some(
                    crate::plist!({
                        "process": { "processIdentifier": pid as i64},
                        "signal": signal as i64,
                    })
                    .into_dictionary()
                    .unwrap(),
                ),
            )
            .await?;

        let res = match plist::from_value(&res) {
            Ok(r) => r,
            Err(e) => {
                warn!("Could not parse signal response: {e:?}");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        Ok(res)
    }

    pub async fn fetch_app_icon(
        &mut self,
        bundle_id: impl Into<String>,
        width: f32,
        height: f32,
        scale: f32,
        allow_placeholder: bool,
    ) -> Result<IconData, IdeviceError> {
        let bundle_id = bundle_id.into();
        let res = self
            .inner
            .invoke(
                "com.apple.coredevice.feature.fetchappicons",
                Some(
                    crate::plist!({
                        "width": width,
                        "height": height,
                        "scale": scale,
                        "allowPlaceholder": allow_placeholder,
                        "bundleIdentifier": bundle_id
                    })
                    .into_dictionary()
                    .unwrap(),
                ),
            )
            .await?;

        let res = match res
            .as_dictionary()
            .and_then(|x| x.get("appIconContainer"))
            .and_then(|x| x.as_dictionary())
            .and_then(|x| x.get("iconImage"))
            .and_then(|x| x.as_data())
        {
            Some(r) => r.to_vec(),
            None => {
                warn!("Did not receive appIconContainer/iconImage data");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        let res = ns_keyed_archive::decode::from_bytes(&res)?;
        match plist::from_value(&res) {
            Ok(r) => Ok(r),
            Err(e) => {
                warn!("Failed to deserialize ns keyed archive: {e:?}");
                Err(IdeviceError::UnexpectedResponse)
            }
        }
    }
}
