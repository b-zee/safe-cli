// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use super::{
    nrs::NRS_MAP_TYPE_TAG,
    xorurl_media_types::{MEDIA_TYPE_CODES, MEDIA_TYPE_STR},
    DEFAULT_XORURL_BASE,
};
use crate::{Error, Result};
use log::{debug, info, warn};
use multibase::{decode, encode, Base};
use safe_nd::{XorName, XOR_NAME_LEN};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::iter::FromIterator;
use tiny_keccak::sha3_256;
use url::Url;

const SAFE_URL_PROTOCOL: &str = "safe://";
const SAFE_URL_SCHEME: &str = "safe";
const XOR_URL_VERSION_1: u64 = 0x1; // TODO: consider using 16 bits
const XOR_URL_STR_MAX_LENGTH: usize = 44;
const XOR_NAME_BYTES_OFFSET: usize = 4; // offset where to find the XoR name bytes
const URL_VERSION_QUERY_NAME: &str = "v";

// The XOR-URL type
pub type XorUrl = String;

// Backwards compatibility for the rest of codebase.
// A later PR will:
//  1. rename this file to safeurl.rs
//  2. change all references in other files
//  3. remove this alias.
pub type XorUrlEncoder = SafeUrl;

// Supported base encoding for XOR URLs
#[derive(Copy, Clone, Debug)]
pub enum XorUrlBase {
    Base32z,
    Base32,
    Base64,
}

impl std::str::FromStr for XorUrlBase {
    type Err = Error;
    fn from_str(str: &str) -> Result<Self> {
        match str {
            "base32z" => Ok(Self::Base32z),
            "base32" => Ok(Self::Base32),
            "base64" => Ok(Self::Base64),
            other => Err(Error::InvalidInput(format!(
                "Invalid XOR URL base encoding: {}. Supported values are base32z, base32, and base64",
                other
            ))),
        }
    }
}

impl fmt::Display for XorUrlBase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl XorUrlBase {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Base32z),
            1 => Ok(Self::Base32),
            2 => Ok(Self::Base64),
            _other => Err(Error::InvalidInput("Invalid XOR URL base encoding code. Supported values are 0=base32z, 1=base32, and 2=base64".to_string())),
        }
    }
}

// We encode the content type that a XOR-URL is targetting, this allows the consumer/user to
// treat the content in particular ways when the content requires it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum SafeContentType {
    Raw,
    Wallet,
    FilesContainer,
    NrsMapContainer,
    MediaType(String),
}

impl std::fmt::Display for SafeContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl SafeContentType {
    pub fn from_u16(value: u16) -> Result<Self> {
        match value {
            0 => Ok(Self::Raw),
            1 => Ok(Self::Wallet),
            2 => Ok(Self::FilesContainer),
            3 => Ok(Self::NrsMapContainer),
            _other => Err(Error::InvalidInput("Invalid Media-type code".to_string())),
        }
    }

    pub fn value(&self) -> Result<u16> {
        match &*self {
            Self::Raw => Ok(0),
            Self::Wallet => Ok(1),
            Self::FilesContainer => Ok(2),
            Self::NrsMapContainer => Ok(3),
            Self::MediaType(media_type) => match MEDIA_TYPE_CODES.get(media_type) {
                Some(media_type_code) => Ok(*media_type_code),
                None => Err(Error::Unexpected("Unsupported Media-type".to_string())),
            },
        }
    }
}

// We also encode the native SAFE data type where the content is being stored on the SAFE Network,
// this allows us to fetch the targetted data using the corresponding API, regardless of the
// data that is being held which is identified by the SafeContentType instead.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum SafeDataType {
    SafeKey = 0x00,
    PublishedImmutableData = 0x01,
    UnpublishedImmutableData = 0x02,
    SeqMutableData = 0x03,
    UnseqMutableData = 0x04,
    PublishedSeqAppendOnlyData = 0x05,
    PublishedUnseqAppendOnlyData = 0x06,
    UnpublishedSeqAppendOnlyData = 0x07,
    UnpublishedUnseqAppendOnlyData = 0x08,
}

impl std::fmt::Display for SafeDataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl SafeDataType {
    pub fn from_u64(value: u64) -> Result<Self> {
        match value {
            0 => Ok(Self::SafeKey),
            1 => Ok(Self::PublishedImmutableData),
            2 => Ok(Self::UnpublishedImmutableData),
            3 => Ok(Self::SeqMutableData),
            4 => Ok(Self::UnseqMutableData),
            5 => Ok(Self::PublishedSeqAppendOnlyData),
            6 => Ok(Self::PublishedUnseqAppendOnlyData),
            7 => Ok(Self::UnpublishedSeqAppendOnlyData),
            8 => Ok(Self::UnpublishedUnseqAppendOnlyData),
            _ => Err(Error::InvalidInput("Invalid SafeDataType code".to_string())),
        }
    }
}

// A simple struct to represent the basic components parsed
// from a Safe URL without any decoding.
//
// This is kept internal to the crate, at least for now.
#[derive(Debug, Clone)]
pub(crate) struct SafeUrlParts {
    pub scheme: String,
    pub host: String,
    pub sub_names: Vec<String>,
    pub tld: String,
    pub path: String,
    pub query_string: String,
    pub fragment: String,
}

impl SafeUrlParts {
    // parses a URL into its component parts, performing basic validation.
    pub fn parse(url: &str) -> Result<Self> {
        let parsing_url = Url::parse(&url).map_err(|parse_err| {
            let msg = format!("Problem parsing the URL \"{}\": {}", url, parse_err);
            Error::InvalidXorUrl(msg)
        })?;

        // Validate the url scheme is 'safe'
        let scheme = parsing_url.scheme();
        if scheme != SAFE_URL_SCHEME {
            let msg = format!(
                "invalid scheme: '{}'. expected: '{}'",
                scheme, SAFE_URL_SCHEME
            );
            return Err(Error::InvalidXorUrl(msg));
        }

        // validate host is not empty
        let host = match parsing_url.host_str() {
            Some(h) => h,
            None => {
                let msg = format!("Problem parsing the URL \"{}\": {}", url, "missing host");
                return Err(Error::InvalidXorUrl(msg));
            }
        };

        // validate no empty sub names in host.
        if host.find("..").is_some() {
            let msg = "host contains empty subname".to_string();
            return Err(Error::InvalidXorUrl(msg));
        }

        // parse tld and sub_names from host
        let names_vec = Vec::from_iter(host.split('.').map(String::from));
        let top_level_name = &names_vec[names_vec.len() - 1];
        let sub_names = &names_vec[0..names_vec.len() - 1];

        // get path, query_params, and fragment
        let path = parsing_url.path();
        let query_params = parsing_url.query().unwrap_or("").to_string();
        let fragment = parsing_url.fragment().unwrap_or("").to_string();

        // double-slash is allowed but discouraged in regular URLs.
        // We don't allow them in Safe URLs.
        // See https://stackoverflow.com/questions/20523318/is-a-url-with-in-the-path-section-valid
        if path.find("//").is_some() {
            let msg = "path contains empty component".to_string();
            return Err(Error::InvalidXorUrl(msg));
        }

        debug!(
            "Parsed url: scheme: {}, host: {}, subnames: {:?}, tld: {}, path: {}, query_string: {}, fragment: {}",
            scheme,
            host,
            sub_names.to_vec(),
            top_level_name.to_string(),
            path,
            query_params,
            fragment,
        );

        let s = Self {
            scheme: scheme.to_string(),
            host: host.to_string(),
            sub_names: sub_names.to_vec(),
            tld: top_level_name.to_string(),
            path: path.to_string(),
            query_string: query_params,
            fragment,
        };

        Ok(s)
    }
}

/// Represents a SafeUrl
///
/// A SafeUrl can be in one of two formats:  nrs or xor.
///   aka:  nrsurl or xorurl
///
// if nrs_host is non-empty, it is considered an nrsurl.
// else an xorurl.  see: SafeUrl::is_nrs()
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafeUrl {
    encoding_version: u64, // currently only v1 supported
    xorname: XorName,      // applies to nrsurl and xorurl
    nrs_host: String,      // full hostname, only for nrsurl
    type_tag: u64,
    data_type: SafeDataType,       // See SafeDataType
    content_type: SafeContentType, // See SafeContentTYpe
    path: String,                  // path, no separator, percent-encoded
    sub_names: Vec<String>,        // only used for xorurl.  tbd: remove?
    query_string: String,          // query-string, no separator, url-encoded
    fragment: String,              // fragment, no separator
    content_version: Option<u64>,  // convenience for ?v=<version
}

/// This implementation performs semi-rigorous validation,
/// when parsing a URL using ::from_url(), ::from_xorurl(),
/// or ::from_nrsurl().
///
/// However setters and new() do not enforce all the rules
/// and using them with invalid input can result in serializing
/// invalid URLs.  GIGO.
///
/// As such, it is recommended to check validity by
/// calling SafeUrl::validate() after instantiating
/// or modifying.
///
// TBD: In the future, we may want to perform all validity
// checks in the setters, however, this requires modifying
// setters to return a Result, which potentially impacts a
// bunch of code elsewhere.
impl SafeUrl {
    #[allow(clippy::too_many_arguments)]
    /// Instantiates a new SafeUrl
    ///
    /// Performs some basic validation checks, however it is
    /// possible to create invalid urls using this method.
    ///
    /// Arguments
    /// * `xorname` - xorname hash
    /// * `nrs_host` - complete nrs hostname, or None for xorurl
    /// * `type_tag` - type tag
    /// * `data_type` - SafeDataType
    /// * `content_type` - SafeContentType
    /// * `path` - must already be percent-encoded if Some. leading '/' optional.
    /// * `xorurl_sub_names` - sub_names. ignored if nrs_host is present.
    /// * `query_string` - must already be percent-encoded, without ? separator
    /// * `fragment` - url fragment, without # separator
    /// * `content_version` - overrides value of "?v" in query-string if not None.
    pub fn new(
        xorname: XorName,
        nrs_host: Option<&str>,
        type_tag: u64,
        data_type: SafeDataType,
        content_type: SafeContentType,
        path: Option<&str>,
        sub_names: Option<Vec<String>>,
        query_string: Option<&str>,
        fragment: Option<&str>,
        content_version: Option<u64>,
    ) -> Result<Self> {
        if let SafeContentType::MediaType(ref media_type) = content_type {
            if !Self::is_media_type_supported(media_type) {
                return Err(Error::InvalidMediaType(format!(
                        "Media-type '{}' not supported. You can use 'SafeContentType::Raw' as the 'content_type' for this type of content",
                        media_type
                    )));
            }
        }

        let host: &str;
        let subnames: Vec<String>;
        match nrs_host {
            Some(nh) => {
                // we have an nrsurl
                if nh.is_empty() {
                    let msg = "nrs_host cannot be empty string.".to_string();
                    return Err(Error::InvalidInput(msg));
                }
                // Validate that nrs_host hash matches xorname
                let tmpurl = format!("{}{}", SAFE_URL_PROTOCOL, nh);
                let parts = SafeUrlParts::parse(&tmpurl)?;
                let hashed_host = Self::xorname_from_nrs_string(&parts.tld)?;
                if hashed_host != xorname {
                    let msg = format!(
                        "input mis-match. nrs_host `{}` does not hash to xorname `{}`",
                        parts.tld, xorname
                    );
                    return Err(Error::InvalidInput(msg));
                }
                host = nh;
                subnames = parts.sub_names; // use sub_names from nrs_host, ignoring sub_names arg, in case they do not match.
            }
            None => {
                // we have an xorurl
                host = "";
                subnames = sub_names.unwrap_or_else(|| vec![]);

                for s in &subnames {
                    if s.is_empty() {
                        let msg = "empty subname".to_string();
                        return Err(Error::InvalidInput(msg));
                    }
                }
            }
        }

        // finally, instantiate.
        let mut x = Self {
            encoding_version: XOR_URL_VERSION_1,
            xorname,
            nrs_host: host.to_string(),
            type_tag,
            data_type,
            content_type,
            path: String::default(), // set below.
            sub_names: subnames,
            query_string: String::default(), // set below.
            fragment: fragment.unwrap_or("").to_string(),
            content_version: None, // set below.
        };

        // we call this to add leading slash if needed
        // but we do NOT want percent-encoding as caller
        // must already provide it that way.
        x.set_path_internal(path.unwrap_or(""), false);

        // we set query_string and content_version using setters to
        // ensure they are in sync.
        x.set_query_string(query_string.unwrap_or(""))?;

        // If present, content_version will override ?v in query string.
        if let Some(version) = content_version {
            x.set_content_version(Some(version));
        }
        Ok(x)
    }

    // A non-member utility function to check if a media-type is currently supported by XOR-URL encoding
    pub fn is_media_type_supported(media_type: &str) -> bool {
        MEDIA_TYPE_CODES.get(media_type).is_some()
    }

    /// Parses a safe url into SafeUrl
    ///
    /// # Arguments
    ///
    /// * `url` - either nrsurl or xorurl
    pub fn from_url(url: &str) -> Result<Self> {
        match Self::from_xorurl(url) {
            Ok(enc) => Ok(enc),
            Err(err) => {
                info!(
                    "Falling back to NRS. XorUrl decoding failed with: {:?}",
                    err
                );
                Self::from_nrsurl(url)
            }
        }
    }

    /// Parses a safe nrsurl into SafeUrl
    ///
    /// # Arguments
    ///
    /// * `nrsurl` - an nrsurl.
    pub fn from_nrsurl(nrsurl: &str) -> Result<Self> {
        let parts = SafeUrlParts::parse(&nrsurl)?;

        let hashed_host = Self::xorname_from_nrs_string(&parts.tld)?;

        let x = Self::new(
            hashed_host,
            Some(&parts.host),
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::NrsMapContainer,
            Some(&parts.path),
            Some(parts.sub_names),
            Some(&parts.query_string),
            Some(&parts.fragment),
            None,
        )?;

        Ok(x)
    }

    /// Parses a safe xorurl into SafeUrl
    ///
    /// # Arguments
    ///
    /// * `xorurl` - an xorurl.
    pub fn from_xorurl(xorurl: &str) -> Result<Self> {
        let parts = SafeUrlParts::parse(&xorurl)?;

        let (_base, xorurl_bytes): (Base, Vec<u8>) = decode(&parts.tld)
            .map_err(|err| Error::InvalidXorUrl(format!("Failed to decode XOR-URL: {:?}", err)))?;

        let type_tag_offset = XOR_NAME_BYTES_OFFSET + XOR_NAME_LEN; // offset where to find the type tag bytes

        // check if too short
        if xorurl_bytes.len() < type_tag_offset {
            return Err(Error::InvalidXorUrl(format!(
                "Invalid XOR-URL, encoded string too short: {} bytes",
                xorurl_bytes.len()
            )));
        }

        // check if too long
        if xorurl_bytes.len() > XOR_URL_STR_MAX_LENGTH {
            return Err(Error::InvalidXorUrl(format!(
                "Invalid XOR-URL, encoded string too long: {} bytes",
                xorurl_bytes.len()
            )));
        }

        // let's make sure we support the XOR_URL version
        let u8_version: u8 = xorurl_bytes[0];
        let encoding_version: u64 = u64::from(u8_version);
        if encoding_version != XOR_URL_VERSION_1 {
            return Err(Error::InvalidXorUrl(format!(
                "Invalid or unsupported XOR-URL encoding version: {}",
                encoding_version
            )));
        }

        let mut content_type_bytes = [0; 2];
        content_type_bytes[0..].copy_from_slice(&xorurl_bytes[1..3]);
        let content_type = match u16::from_be_bytes(content_type_bytes) {
            0 => SafeContentType::Raw,
            1 => SafeContentType::Wallet,
            2 => SafeContentType::FilesContainer,
            3 => SafeContentType::NrsMapContainer,
            other => match MEDIA_TYPE_STR.get(&other) {
                Some(media_type_str) => SafeContentType::MediaType((*media_type_str).to_string()),
                None => {
                    return Err(Error::InvalidXorUrl(format!(
                        "Invalid content type encoded in the XOR-URL string: {}",
                        other
                    )))
                }
            },
        };

        debug!(
            "Attempting to match content type of URL: {}, {:?}",
            &xorurl, content_type
        );

        let data_type = match xorurl_bytes[3] {
            0 => SafeDataType::SafeKey,
            1 => SafeDataType::PublishedImmutableData,
            2 => SafeDataType::UnpublishedImmutableData,
            3 => SafeDataType::SeqMutableData,
            4 => SafeDataType::UnseqMutableData,
            5 => SafeDataType::PublishedSeqAppendOnlyData,
            6 => SafeDataType::PublishedUnseqAppendOnlyData,
            7 => SafeDataType::UnpublishedSeqAppendOnlyData,
            8 => SafeDataType::UnpublishedUnseqAppendOnlyData,
            other => {
                return Err(Error::InvalidXorUrl(format!(
                    "Invalid SAFE data type encoded in the XOR-URL string: {}",
                    other
                )))
            }
        };

        let mut xorname = XorName::default();
        xorname
            .0
            .copy_from_slice(&xorurl_bytes[XOR_NAME_BYTES_OFFSET..type_tag_offset]);

        let type_tag_bytes_len = xorurl_bytes.len() - type_tag_offset;

        let mut type_tag_bytes = [0; 8];
        type_tag_bytes[8 - type_tag_bytes_len..].copy_from_slice(&xorurl_bytes[type_tag_offset..]);
        let type_tag: u64 = u64::from_be_bytes(type_tag_bytes);

        let x = Self::new(
            xorname,
            None, // no nrs_host for an xorurl
            type_tag,
            data_type,
            content_type,
            Some(&parts.path),
            Some(parts.sub_names),
            Some(&parts.query_string),
            Some(&parts.fragment),
            None,
        )?;

        Ok(x)
    }

    /// The url scheme.  Only 'safe' scheme is presently supported.
    pub fn scheme(&self) -> &str {
        SAFE_URL_SCHEME
    }

    /// returns encoding version of xorurl
    pub fn encoding_version(&self) -> u64 {
        self.encoding_version
    }

    /// returns SAFE data type
    pub fn data_type(&self) -> SafeDataType {
        self.data_type.clone()
    }

    /// returns SAFE content type
    pub fn content_type(&self) -> SafeContentType {
        self.content_type.clone()
    }

    /// returns XorName
    pub fn xorname(&self) -> XorName {
        self.xorname
    }

    /// returns 'host' portion of xorurl using the
    /// default xorurl encoding.
    ///
    /// For a different encoding, see host_to_base()
    pub fn xorurl_host(&self) -> String {
        self.host_to_base(DEFAULT_XORURL_BASE).unwrap_or_else(|e| {
            warn!("{}", e);
            String::default()
        })
    }

    /// returns nrs_host
    ///
    /// Will be empty string if is_nrs() != true
    pub fn nrs_host(&self) -> &str {
        &self.nrs_host
    }

    /// The url host.  Either nrs_host or xorname.
    pub fn host(&self) -> String {
        if self.is_nrs() {
            self.nrs_host.clone()
        } else {
            self.xorurl_host()
        }
    }

    /// returns top-level-domain of host field.
    ///
    /// eg: my.sub.name --> name
    pub fn tld(&self) -> String {
        let host = self.host();
        let parts: Vec<&str> = host.split('.').collect();
        let default = "";
        (*parts.last().unwrap_or(&default)).to_string()
    }

    /// returns XorUrl type tag
    pub fn type_tag(&self) -> u64 {
        self.type_tag
    }

    /// returns path portion of URL, percent encoded (unmodified).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// returns path portion of URL, percent decoded
    pub fn path_decoded(&self) -> Result<String> {
        Self::url_percent_decode(&self.path)
    }

    /// sets path portion of URL
    ///
    /// input string must not be percent-encoded.
    /// The encoding is done internally.
    ///
    /// leading slash is automatically added if necessary.
    pub fn set_path(&mut self, path: &str) {
        self.set_path_internal(path, true);
    }

    /// returns nrs sub_names
    pub fn sub_names(&self) -> Vec<String> {
        self.sub_names.to_vec()
    }

    /// gets content version
    ///
    /// This is a shortcut method for getting the "?v=" query param.
    pub fn content_version(&self) -> Option<u64> {
        self.content_version
    }

    /// sets content version
    ///
    /// This is a shortcut method for setting the "?v=" query param.
    ///
    /// # Arguments
    ///
    /// * `version` - u64 representing value of ?v=<val>
    pub fn set_content_version(&mut self, version: Option<u64>) {
        // Convert Option<u64> to Option<&str>
        let version_string: String;
        let v_option = match version {
            Some(v) => {
                version_string = v.to_string();
                Some(version_string.as_str())
            }
            None => None,
        };

        // note: We are being passed a u64
        // which logically should never fail to be set.  Details of
        // this implementation presently require parsing the query
        // string, but that could change in the future without API changing.
        // eg: by storing parsed key/val pairs.
        // Parsing of the query string is checked/validated by
        // set_query_string().  Thus, it should never be invalid, else
        // we have a serious bug in SafeUrl impl.
        self.set_query_key(URL_VERSION_QUERY_NAME, v_option)
            .unwrap_or_else(|e| {
                warn!("{}", e);
            });
    }

    /// sets or unsets a key/val pair in query string.
    ///
    /// if val is Some, then key=val will be set in query string.
    ///    If there is more than one instance of key in query string,
    ///    there will be only one after this call.
    /// If val is None, then the key will be removed from query string.
    ///
    /// To set key without any value, pass Some<""> as the val.
    ///
    /// `val` should not be percent-encoded.  That is done internally.
    ///
    /// # Arguments
    ///
    /// * `key` - name of url query string var
    /// * `val` - an option representing the value, or none.
    pub fn set_query_key(&mut self, key: &str, val: Option<&str>) -> Result<()> {
        let mut url = Self::query_string_to_url(&self.query_string)?;
        let url2 = url.clone();
        let mut pairs = url.query_pairs_mut();
        pairs.clear();

        let mut set_key = false;
        for (k, v) in url2.query_pairs() {
            if k == key {
                // note: this will consolidate multiple ?k= into just one.
                if let Some(v) = val {
                    if !set_key {
                        pairs.append_pair(key, v);
                        set_key = true;
                    }
                }
            } else {
                pairs.append_pair(&k, &v);
            }
        }
        if !set_key {
            if let Some(v) = val {
                pairs.append_pair(key, v);
            }
        }
        std::mem::drop(pairs);

        self.query_string = url.query().unwrap_or("").to_string();
        debug!("Set query_string: {}", self.query_string);

        if key == URL_VERSION_QUERY_NAME {
            self.set_content_version_internal(val)?;
        }
        Ok(())
    }

    /// sets query string.
    ///
    /// If the query string contains ?v=<version> then it
    /// will take effect as if set_content_version() had been
    /// called.
    ///
    /// # Arguments
    ///
    /// * `query` - percent-encoded key/val pairs.
    pub fn set_query_string(&mut self, query: &str) -> Result<()> {
        // ?v is a special case, so if it is contained in query string
        // we parse it and update our stored content_version.
        // tbd: another option could be to throw an error if input
        // contains ?v.
        let v_option = Self::query_key_last_internal(query, URL_VERSION_QUERY_NAME);
        self.set_content_version_internal(v_option.as_deref())?;

        self.query_string = query.to_string();
        Ok(())
    }

    /// Retrieves query string
    ///
    /// This contains the percent-encoded key/value pairs
    /// as seen in a url.
    pub fn query_string(&self) -> String {
        self.query_string.clone()
    }

    /// Retrieves query string, with ? separator if non-empty.
    pub fn query_string_with_separator(&self) -> String {
        let qs = self.query_string();
        if qs.is_empty() {
            qs
        } else {
            format!("?{}", qs)
        }
    }

    /// Retrieves all query pairs, percent-decoded.
    pub fn query_pairs(&self) -> Vec<(String, String)> {
        Self::query_pairs_internal(&self.query_string)
    }

    /// Queries a key from the query string.
    ///
    /// Can return 0, 1, or many values because a given key
    /// may exist 0, 1, or many times in a URL query-string.
    pub fn query_key(&self, key: &str) -> Vec<String> {
        Self::query_key_internal(&self.query_string, key)
    }

    /// returns the last matching key from a query string.
    ///
    /// eg in safe://name?color=red&age=5&color=green&color=blue
    ///    blue would be returned when key is "color".
    pub fn query_key_last(&self, key: &str) -> Option<String> {
        Self::query_key_last_internal(&self.query_string, key)
    }

    /// returns the first matching key from a query string.
    ///
    /// eg in safe://name?color=red&age=5&color=green&color=blue
    ///    red would be returned when key is "color".
    pub fn query_key_first(&self, key: &str) -> Option<String> {
        Self::query_key_first_internal(&self.query_string, key)
    }

    /// sets url fragment
    pub fn set_fragment(&mut self, fragment: String) {
        self.fragment = fragment;
    }

    /// Retrieves url fragment, without # separator
    pub fn fragment(&self) -> String {
        self.fragment.clone()
    }

    /// Retrieves url fragment, with # separator if non-empty.
    pub fn fragment_with_separator(&self) -> String {
        if self.fragment.is_empty() {
            "".to_string()
        } else {
            format!("#{}", self.fragment)
        }
    }

    /// returns true if an NrsUrl, false if an XorUrl
    pub fn is_nrs(&self) -> bool {
        !self.nrs_host.is_empty()
    }

    // XOR-URL encoding format (var length from 36 to 44 bytes):
    // 1 byte for encoding version
    // 2 bytes for content type (enough to start including some MIME types also)
    // 1 byte for SAFE native data type
    // 32 bytes for XoR Name
    // and up to 8 bytes for type_tag
    // query param "v=" is treated as the content version

    /// serializes the URL to an XorUrl string.
    ///
    /// This function may be called on an NrsUrl and
    /// the corresponding XorUrl will be returned.
    pub fn to_xorurl_string(&self) -> String {
        self.to_base(DEFAULT_XORURL_BASE).unwrap_or_else(|e| {
            warn!("{}", e);
            String::default()
        })
    }

    /// serializes the URL to an NrsUrl string.
    ///
    /// This function returns None when is_nrs() is false.
    pub fn to_nrsurl_string(&self) -> Option<String> {
        if !self.is_nrs() {
            return None;
        }

        let query_string = self.query_string_with_separator();
        let fragment = self.fragment_with_separator();

        let url = format!(
            "{}{}{}{}{}",
            SAFE_URL_PROTOCOL, self.nrs_host, self.path, query_string, fragment
        );
        Some(url)
    }

    /// serializes entire xorurl using a particular base encoding.
    pub fn to_base(&self, base: XorUrlBase) -> Result<String> {
        let host = self.host_to_base(base)?;

        let query_string = self.query_string_with_separator();
        let fragment = self.fragment_with_separator();

        let xorurl = format!(
            "{}{}{}{}{}",
            SAFE_URL_PROTOCOL, host, self.path, query_string, fragment
        );

        Ok(xorurl)
    }

    /// serializes host portion of xorurl using a particular base encoding.
    pub fn host_to_base(&self, base: XorUrlBase) -> Result<String> {
        // let's set the first byte with the XOR-URL format version
        let mut cid_vec: Vec<u8> = vec![XOR_URL_VERSION_1 as u8];

        // add the content type bytes
        let content_type: u16 = match &self.content_type {
            SafeContentType::Raw => 0x0000,
            SafeContentType::Wallet => 0x0001,
            SafeContentType::FilesContainer => 0x0002,
            SafeContentType::NrsMapContainer => 0x0003,
            SafeContentType::MediaType(media_type) => match MEDIA_TYPE_CODES.get(media_type) {
                Some(media_type_code) => *media_type_code,
                None => {
                    return Err(Error::Unexpected(format!(
                        "Failed to encode Media-type '{}'",
                        media_type
                    )))
                }
            },
        };
        cid_vec.extend_from_slice(&content_type.to_be_bytes());

        // push the SAFE data type byte
        cid_vec.push(self.data_type.clone() as u8);

        // add the xorname 32 bytes
        cid_vec.extend_from_slice(&self.xorname.0);

        // let's get non-zero bytes only from th type_tag
        let start_byte: usize = (self.type_tag.leading_zeros() / 8) as usize;
        // add the non-zero bytes of type_tag
        cid_vec.extend_from_slice(&self.type_tag.to_be_bytes()[start_byte..]);

        let base_encoding = match base {
            XorUrlBase::Base32z => Base::Base32z,
            XorUrlBase::Base32 => Base::Base32,
            XorUrlBase::Base64 => Base::Base64,
        };
        let tld = encode(base_encoding, cid_vec);

        // TBD: I'd like to get rid of these sub-names for xorurls.
        // They are ugly and mash 2 distinct concepts together.
        // I compare it to saying subdomain.196.318.5.189
        // The mind rebels...
        let sub_names = if !self.sub_names.is_empty() {
            format!("{}.", self.sub_names.join("."))
        } else {
            "".to_string()
        };

        let host = format!("{}{}", sub_names, tld);

        Ok(host)
    }

    /// Utility function to perform url percent decoding.
    pub fn url_percent_decode(s: &str) -> Result<String> {
        match urlencoding::decode(s) {
            Ok(c) => Ok(c),
            Err(e) => Err(Error::InvalidInput(format!("{:#?}", e))),
        }
    }

    /// Utility function to perform url percent encoding.
    pub fn url_percent_encode(s: &str) -> String {
        urlencoding::encode(s)
    }

    /// Validates that a SafeUrl instance can be parsed correctly.
    ///
    /// SafeUrl::from_url() performs rigorous validation,
    /// however setters and new() do not enforce all the rules
    ///
    /// This routine enables a caller to easily validate
    /// that the present instance passes all validation checks
    pub fn validate(&self) -> Result<()> {
        let s = self.to_string();
        match Self::from_url(&s) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    // A non-member encoder function for convenience in some cases
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        xorname: XorName,
        nrs_host: Option<&str>,
        type_tag: u64,
        data_type: SafeDataType,
        content_type: SafeContentType,
        path: Option<&str>,
        sub_names: Option<Vec<String>>,
        query_string: Option<&str>,
        fragment: Option<&str>,
        content_version: Option<u64>,
        base: XorUrlBase,
    ) -> Result<String> {
        let xorurl_encoder = SafeUrl::new(
            xorname,
            nrs_host,
            type_tag,
            data_type,
            content_type,
            path,
            sub_names,
            query_string,
            fragment,
            content_version,
        )?;
        xorurl_encoder.to_base(base)
    }

    // A non-member SafeKey encoder function for convenience
    pub fn encode_safekey(xorname: XorName, base: XorUrlBase) -> Result<String> {
        SafeUrl::encode(
            xorname,
            None,
            0,
            SafeDataType::SafeKey,
            SafeContentType::Raw,
            None,
            None,
            None,
            None,
            None,
            base,
        )
    }

    // A non-member ImmutableData encoder function for convenience
    pub fn encode_immutable_data(
        xorname: XorName,
        content_type: SafeContentType,
        base: XorUrlBase,
    ) -> Result<String> {
        SafeUrl::encode(
            xorname,
            None,
            0,
            SafeDataType::PublishedImmutableData,
            content_type,
            None,
            None,
            None,
            None,
            None,
            base,
        )
    }

    // A non-member MutableData encoder function for convenience
    pub fn encode_mutable_data(
        xorname: XorName,
        type_tag: u64,
        content_type: SafeContentType,
        base: XorUrlBase,
    ) -> Result<String> {
        SafeUrl::encode(
            xorname,
            None,
            type_tag,
            SafeDataType::SeqMutableData,
            content_type,
            None,
            None,
            None,
            None,
            None,
            base,
        )
    }

    // A non-member AppendOnlyData encoder function for convenience
    pub fn encode_append_only_data(
        xorname: XorName,
        type_tag: u64,
        content_type: SafeContentType,
        base: XorUrlBase,
    ) -> Result<String> {
        SafeUrl::encode(
            xorname,
            None,
            type_tag,
            SafeDataType::PublishedSeqAppendOnlyData,
            content_type,
            None,
            None,
            None,
            None,
            None,
            base,
        )
    }

    // utility to generate a dummy url from a query string.
    fn query_string_to_url(query_str: &str) -> Result<Url> {
        let dummy = format!("file://dummy?{}", query_str);
        match Url::parse(&dummy) {
            Ok(u) => Ok(u),
            Err(_e) => {
                let msg = format!("Invalid query string: {}", query_str);
                Err(Error::InvalidInput(msg))
            }
        }
    }

    // utility to retrieve all unescaped key/val pairs from query string.
    fn query_pairs_internal(query_str: &str) -> Vec<(String, String)> {
        let url = match Self::query_string_to_url(query_str) {
            Ok(u) => u,
            Err(_) => {
                return Vec::<(String, String)>::new();
            }
        };

        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        pairs
    }

    // sets content_version property.
    //
    // This should never be called directly.
    // Use ::set_content_version() or ::set_query_key() instead.
    fn set_content_version_internal(&mut self, version_option: Option<&str>) -> Result<()> {
        if let Some(version_str) = version_option {
            let version = version_str.parse::<u64>().map_err(|_e| {
                let msg = format!(
                    "{} param could not be parsed as u64. invalid: '{}'",
                    URL_VERSION_QUERY_NAME, version_str
                );
                Error::InvalidInput(msg)
            })?;
            self.content_version = Some(version);
        } else {
            self.content_version = None;
        }
        debug!("Set version: {:#?}", self.content_version);
        Ok(())
    }

    // sets path portion of URL
    //
    // input path may be percent-encoded or not, but
    // percent_encode param must be set appropriately
    // to avoid not-encoded or double-encoded isues.
    //
    // leading slash is automatically added if necessary.
    fn set_path_internal(&mut self, path: &str, percent_encode: bool) {
        // fast path for empty string.
        if path.is_empty() {
            if !self.path.is_empty() {
                self.path = path.to_string();
            }
            return;
        }

        // impl note: this func tries to behave like url::Url::set_path()
        // with respect to percent-encoding each path component.
        //
        // tbd: It might be more correct to simply instantiate a
        // dummy URL and call set_path(), return path();
        // counter-argument is that Url::set_path() does not
        // prefix leading slash and allows urls to be created
        // that merge host and path together.
        let parts: Vec<&str> = path.split('/').collect();
        let mut new_parts = Vec::<String>::new();
        for (count, p) in parts.into_iter().enumerate() {
            if !p.is_empty() || count > 0 {
                if percent_encode {
                    new_parts.push(Self::url_percent_encode(p));
                } else {
                    new_parts.push(p.to_string());
                }
            }
        }
        let new_path = new_parts.join("/");

        let separator = if new_path.is_empty() { "" } else { "/" };
        self.path = format!("{}{}", separator, new_path);
    }

    // utility to query a key from a query string, percent-decoded.
    // Can return 0, 1, or many values because a given key
    // can exist 0, 1, or many times in a URL query-string.
    fn query_key_internal(query_str: &str, key: &str) -> Vec<String> {
        let pairs = Self::query_pairs_internal(query_str);
        let mut values = Vec::<String>::new();

        for (k, val) in pairs {
            if k == key {
                values.push(val);
            }
        }
        values
    }

    // utility to query a key from a query string, percent-decoded.
    // returns the last matching key.
    // eg in safe://name?color=red&age=5&color=green&color=blue
    //    blue would be returned when key is "color".
    fn query_key_last_internal(query_str: &str, key: &str) -> Option<String> {
        let matches = Self::query_key_internal(query_str, key);
        match matches.last() {
            Some(v) => Some(v.to_string()),
            None => None,
        }
    }

    // utility to query a key from a query string.
    // returns the last matching key.
    // eg in safe://name?color=red&age=5&color=green&color=blue
    //    blue would be returned when key is "color".
    fn query_key_first_internal(query_str: &str, key: &str) -> Option<String> {
        let matches = Self::query_key_internal(query_str, key);
        match matches.first() {
            Some(v) => Some(v.to_string()),
            None => None,
        }
    }

    fn xorname_from_nrs_string(name: &str) -> Result<XorName> {
        let vec_hash = sha3_256(&name.to_string().into_bytes());
        let xorname = XorName(vec_hash);
        debug!("Resulting XorName for NRS \"{}\" is: {}", name, xorname);
        Ok(xorname)
    }
}

impl fmt::Display for SafeUrl {
    /// serializes the URL to a string.
    ///
    /// an NrsUrl will be serialized in NrsUrl form.
    /// an XorUrl will be serialized in XorUrl form.
    ///
    /// See also:
    ///  * ::to_xorurl_string()
    ///  * ::to_nrs_url_string()
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let buf = if self.is_nrs() {
            match self.to_nrsurl_string() {
                Some(s) => s,
                None => {
                    warn!("to_nrsurl_string() return None when is_nrs() == true. '{}'.  This should never happen. Please investigate.", self.nrs_host);
                    return Err(fmt::Error);
                }
            }
        } else {
            self.to_xorurl_string()
        };
        write!(fmt, "{}", buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safeurl_new_validation() -> Result<()> {
        // Tests some errors when calling Self::new()

        let msg = "Expected error";
        let wrong_err = "Wrong error type";

        let xorname = XorName(*b"12345678901234567890123456789012");

        // test: "Media-type '{}' not supported. You can use 'SafeContentType::Raw' as the 'content_type' for this type of content",
        let result = SafeUrl::new(
            xorname,
            None,
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::MediaType("garbage/trash".to_string()),
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err(msg);
        match result {
            Error::InvalidMediaType(e) => assert!(e.contains("You can use 'SafeContentType::Raw'")),
            _ => panic!(wrong_err),
        }

        // test: "nrs_host cannot be empty string."
        let result = SafeUrl::new(
            xorname,
            Some(""), // passing empty string as nrs host
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::NrsMapContainer,
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err(msg);
        match result {
            Error::InvalidInput(e) => assert!(e.contains("nrs_host cannot be empty string.")),
            _ => panic!(wrong_err),
        }

        // test: "input mis-match. nrs_host `{}` does not hash to xorname `{}`"
        let result = SafeUrl::new(
            xorname,
            Some("a.b.c"), // passing nrs host not matching xorname.
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::NrsMapContainer,
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err(msg);
        match result {
            Error::InvalidInput(e) => assert!(e.contains("does not hash to xorname")),
            _ => panic!(wrong_err),
        }

        // test: "Host contains empty subname" (in nrs host)
        let result = SafeUrl::new(
            xorname,
            Some("a..b.c"), // passing empty sub-name in nrs host
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::NrsMapContainer,
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("host contains empty subname")),
            _ => panic!(wrong_err),
        }

        // test: "empty subname" (in xorurl sub_names)
        let result = SafeUrl::new(
            xorname,
            None, // not NRS
            NRS_MAP_TYPE_TAG,
            SafeDataType::PublishedSeqAppendOnlyData,
            SafeContentType::NrsMapContainer,
            None,
            Some(vec!["a".to_string(), "".to_string(), "b".to_string()]),
            None,
            None,
            None,
        )
        .expect_err(msg);
        match result {
            Error::InvalidInput(e) => assert!(e.contains("empty subname")),
            _ => panic!(wrong_err),
        }

        Ok(())
    }

    #[test]
    fn test_safeurl_base32_encoding() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let xorurl = SafeUrl::encode(
            xorname,
            None,
            0xa632_3c4d_4a32,
            SafeDataType::PublishedImmutableData,
            SafeContentType::Raw,
            None,
            None,
            None,
            None,
            None,
            XorUrlBase::Base32,
        )?;
        let base32_xorurl =
            "safe://biaaaatcmrtgq2tmnzyheydcmrtgq2tmnzyheydcmrtgq2tmnzyheydcmvggi6e2srs";
        assert_eq!(xorurl, base32_xorurl);
        Ok(())
    }

    #[test]
    fn test_safeurl_base32z_encoding() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let xorurl =
            SafeUrl::encode_immutable_data(xorname, SafeContentType::Raw, XorUrlBase::Base32z)?;
        let base32z_xorurl = "safe://hbyyyyncj1gc4dkptz8yhuycj1gc4dkptz8yhuycj1gc4dkptz8yhuycj1";
        assert_eq!(xorurl, base32z_xorurl);
        Ok(())
    }

    #[test]
    fn test_safeurl_base64_encoding() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let xorurl = SafeUrl::encode_append_only_data(
            xorname,
            4_584_545,
            SafeContentType::FilesContainer,
            XorUrlBase::Base64,
        )?;
        let base64_xorurl = "safe://mQACBTEyMzQ1Njc4OTAxMjM0NTY3ODkwMTIzNDU2Nzg5MDEyRfRh";
        assert_eq!(xorurl, base64_xorurl);
        let xorurl_encoder = SafeUrl::from_url(&base64_xorurl)?;
        assert_eq!(base64_xorurl, xorurl_encoder.to_base(XorUrlBase::Base64)?);
        assert_eq!("", xorurl_encoder.path());
        assert_eq!(XOR_URL_VERSION_1, xorurl_encoder.encoding_version());
        assert_eq!(xorname, xorurl_encoder.xorname());
        assert_eq!(4_584_545, xorurl_encoder.type_tag());
        assert_eq!(
            SafeDataType::PublishedSeqAppendOnlyData,
            xorurl_encoder.data_type()
        );
        assert_eq!(
            SafeContentType::FilesContainer,
            xorurl_encoder.content_type()
        );
        Ok(())
    }

    #[test]
    fn test_safeurl_default_base_encoding() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let base32z_xorurl = "safe://hbyyyyncj1gc4dkptz8yhuycj1gc4dkptz8yhuycj1gc4dkptz8yhuycj1";
        let xorurl =
            SafeUrl::encode_immutable_data(xorname, SafeContentType::Raw, DEFAULT_XORURL_BASE)?;
        assert_eq!(xorurl, base32z_xorurl);
        Ok(())
    }

    #[test]
    fn test_safeurl_decoding() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let type_tag: u64 = 0x0eef;
        let subdirs = "/dir1/dir2";
        let content_version = 5;
        let query_string = "k1=v1&k2=v2";
        let query_string_v = format!("{}&v={}", query_string, content_version);
        let fragment = "myfragment";
        let xorurl = SafeUrl::encode(
            xorname,
            None,
            type_tag,
            SafeDataType::PublishedImmutableData,
            SafeContentType::Raw,
            Some(subdirs),
            Some(vec!["subname".to_string()]),
            Some(query_string),
            Some(fragment),
            Some(5),
            XorUrlBase::Base32z,
        )?;
        let xorurl_encoder = SafeUrl::from_url(&xorurl)?;

        assert_eq!(subdirs, xorurl_encoder.path());
        assert_eq!(XOR_URL_VERSION_1, xorurl_encoder.encoding_version());
        assert_eq!(xorname, xorurl_encoder.xorname());
        assert_eq!(type_tag, xorurl_encoder.type_tag());
        assert_eq!(
            SafeDataType::PublishedImmutableData,
            xorurl_encoder.data_type()
        );
        assert_eq!(SafeContentType::Raw, xorurl_encoder.content_type());
        assert_eq!(Some(content_version), xorurl_encoder.content_version());
        assert_eq!(query_string_v, xorurl_encoder.query_string());
        assert_eq!(fragment, xorurl_encoder.fragment());
        Ok(())
    }

    #[test]
    fn test_safeurl_decoding_with_path() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let type_tag: u64 = 0x0eef;
        let xorurl = SafeUrl::encode_append_only_data(
            xorname,
            type_tag,
            SafeContentType::Wallet,
            XorUrlBase::Base32z,
        )?;

        let xorurl_with_path = format!("{}/subfolder/file", xorurl);
        let xorurl_encoder_with_path = SafeUrl::from_url(&xorurl_with_path)?;
        assert_eq!(
            xorurl_with_path,
            xorurl_encoder_with_path.to_base(XorUrlBase::Base32z)?
        );
        assert_eq!("/subfolder/file", xorurl_encoder_with_path.path());
        assert_eq!(
            XOR_URL_VERSION_1,
            xorurl_encoder_with_path.encoding_version()
        );
        assert_eq!(xorname, xorurl_encoder_with_path.xorname());
        assert_eq!(type_tag, xorurl_encoder_with_path.type_tag());
        assert_eq!(
            SafeDataType::PublishedSeqAppendOnlyData,
            xorurl_encoder_with_path.data_type()
        );
        assert_eq!(
            SafeContentType::Wallet,
            xorurl_encoder_with_path.content_type()
        );
        Ok(())
    }

    #[test]
    fn test_safeurl_decoding_with_subname() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let type_tag: u64 = 0x0eef;
        let xorurl_with_subname = SafeUrl::encode(
            xorname,
            None,
            type_tag,
            SafeDataType::PublishedImmutableData,
            SafeContentType::NrsMapContainer,
            None,
            Some(vec!["sub".to_string()]),
            None,
            None,
            None,
            XorUrlBase::Base32z,
        )?;

        assert!(xorurl_with_subname.contains("safe://sub."));
        let xorurl_encoder_with_subname = SafeUrl::from_url(&xorurl_with_subname)?;
        assert_eq!(
            xorurl_with_subname,
            xorurl_encoder_with_subname.to_base(XorUrlBase::Base32z)?
        );
        assert_eq!("", xorurl_encoder_with_subname.path());
        assert_eq!(1, xorurl_encoder_with_subname.encoding_version());
        assert_eq!(xorname, xorurl_encoder_with_subname.xorname());
        assert_eq!(type_tag, xorurl_encoder_with_subname.type_tag());
        assert_eq!(vec!("sub"), xorurl_encoder_with_subname.sub_names());
        assert_eq!(
            SafeContentType::NrsMapContainer,
            xorurl_encoder_with_subname.content_type()
        );
        Ok(())
    }

    #[test]
    fn test_safeurl_encoding_decoding_with_media_type() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let xorurl = SafeUrl::encode_immutable_data(
            xorname,
            SafeContentType::MediaType("text/html".to_string()),
            XorUrlBase::Base32z,
        )?;

        let xorurl_encoder = SafeUrl::from_url(&xorurl)?;
        assert_eq!(
            SafeContentType::MediaType("text/html".to_string()),
            xorurl_encoder.content_type()
        );
        Ok(())
    }

    #[test]
    fn test_safeurl_too_long() -> Result<()> {
        let xorurl =
            "safe://heyyynunctugo4ucp3a8radnctugo4ucp3a8radnctugo4ucp3a8radnctmfp5zq75zq75zq7";

        match SafeUrl::from_xorurl(xorurl) {
            Ok(_) => Err(Error::Unexpected(
                "Unexpectedly parsed an invalid (too long) xorurl".to_string(),
            )),
            Err(Error::InvalidXorUrl(msg)) => {
                assert!(msg.starts_with("Invalid XOR-URL, encoded string too long"));
                Ok(())
            }
            other => Err(Error::Unexpected(format!(
                "Error returned is not the expected one: {:?}",
                other
            ))),
        }
    }

    #[test]
    fn test_safeurl_too_short() -> Result<()> {
        let xorname = XorName(*b"12345678901234567890123456789012");
        let xorurl = SafeUrl::encode_immutable_data(
            xorname,
            SafeContentType::MediaType("text/html".to_string()),
            XorUrlBase::Base32z,
        )?;

        let len = xorurl.len() - 1;
        match SafeUrl::from_xorurl(&xorurl[..len]) {
            Ok(_) => Err(Error::Unexpected(
                "Unexpectedly parsed an invalid (too short) xorurl".to_string(),
            )),
            Err(Error::InvalidXorUrl(msg)) => {
                assert!(msg.starts_with("Invalid XOR-URL, encoded string too short"));
                Ok(())
            }
            other => Err(Error::Unexpected(format!(
                "Error returned is not the expected one: {:?}",
                other
            ))),
        }
    }

    #[test]
    fn test_safeurl_query_key_first() -> Result<()> {
        let x = SafeUrl::from_url("safe://myname?name=John+Doe&name=Jane%20Doe")?;
        let name = x.query_key_first("name");
        assert_eq!(name, Some("John Doe".to_string()));

        Ok(())
    }

    #[test]
    fn test_safeurl_query_key_last() -> Result<()> {
        let x = SafeUrl::from_url("safe://myname?name=John+Doe&name=Jane%20Doe")?;
        let name = x.query_key_last("name");
        assert_eq!(name, Some("Jane Doe".to_string()));

        Ok(())
    }

    #[test]
    fn test_safeurl_query_key() -> Result<()> {
        let x = SafeUrl::from_url("safe://myname?name=John+Doe&name=Jane%20Doe")?;
        let name = x.query_key("name");
        assert_eq!(name, vec!["John Doe".to_string(), "Jane Doe".to_string()]);

        Ok(())
    }

    #[test]
    fn test_safeurl_set_query_key() -> Result<()> {
        let mut x = SafeUrl::from_url("safe://myname?name=John+Doe&name=Jane%20Doe")?;

        // set_query_key should replace the multiple name= with a single instance.
        let peggy_sue = "Peggy Sue".to_string();
        x.set_query_key("name", Some(&peggy_sue))?;
        assert_eq!(x.query_key_first("name"), Some(peggy_sue.clone()));
        assert_eq!(x.query_key_last("name"), Some(peggy_sue));
        assert_eq!(x.to_string(), "safe://myname?name=Peggy+Sue");

        // None should remove the name param.
        x.set_query_key("name", None)?;
        assert_eq!(x.query_key_last("name"), None);
        assert_eq!(x.to_string(), "safe://myname");

        // Test setting an empty key.
        x.set_query_key("name", Some(""))?;
        x.set_query_key("age", Some("25"))?;
        assert_eq!(x.query_key_last("name"), Some("".to_string()));
        assert_eq!(x.query_key_last("age"), Some("25".to_string()));
        assert_eq!(x.to_string(), "safe://myname?name=&age=25");

        // Test setting content version via ?v=61342
        x.set_query_key(URL_VERSION_QUERY_NAME, Some("61342"))?;
        assert_eq!(
            x.query_key_last(URL_VERSION_QUERY_NAME),
            Some("61342".to_string())
        );
        assert_eq!(x.content_version(), Some(61342));

        // Test unsetting content version via ?v=None
        x.set_query_key(URL_VERSION_QUERY_NAME, None)?;
        assert_eq!(x.query_key_last(URL_VERSION_QUERY_NAME), None);
        assert_eq!(x.content_version(), None);

        // Test parse error for version via ?v=non-integer
        let result = x.set_query_key(URL_VERSION_QUERY_NAME, Some("non-integer"));
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_safeurl_set_content_version() -> Result<()> {
        let mut x = SafeUrl::from_url("safe://myname?name=John+Doe&name=Jane%20Doe")?;

        x.set_content_version(Some(234));
        assert_eq!(
            x.query_key_first(URL_VERSION_QUERY_NAME),
            Some("234".to_string())
        );
        assert_eq!(x.content_version(), Some(234));
        assert_eq!(
            x.to_string(),
            "safe://myname?name=John+Doe&name=Jane+Doe&v=234"
        );

        x.set_content_version(None);
        assert_eq!(x.query_key_first(URL_VERSION_QUERY_NAME), None);
        assert_eq!(x.content_version(), None);
        assert_eq!(x.to_string(), "safe://myname?name=John+Doe&name=Jane+Doe");

        Ok(())
    }

    #[test]
    fn test_safeurl_path() -> Result<()> {
        // Make sure we can read percent-encoded paths, and set them as well.
        let mut x = SafeUrl::from_url("safe://domain/path/to/my%20file.txt?v=1")?;
        assert_eq!(x.path(), "/path/to/my%20file.txt");
        x.set_path("/path/to/my new file.txt");
        assert_eq!(x.path(), "/path/to/my%20new%20file.txt");
        assert_eq!(x.path_decoded()?, "/path/to/my new file.txt");
        x.set_path("/trailing/slash/");
        assert_eq!(x.path(), "/trailing/slash/");

        // here we verify that url::Url has the same path encoding behavior
        // as our implementation.  for better or worse.
        let mut u = Url::parse("safe://domain/path/to/my%20file.txt?v=1").unwrap();
        assert_eq!(u.path(), "/path/to/my%20file.txt");
        u.set_path("/path/to/my new file.txt");
        assert_eq!(u.path(), "/path/to/my%20new%20file.txt");
        u.set_path("/trailing/slash/");
        assert_eq!(u.path(), "/trailing/slash/");

        // note: our impl and url::Url differ with no-leading-slash behavior.
        // we prepend leading slash when storing and return a changed path.
        // some SAFE code appears to depend on this presently.
        x.set_path("no-leading-slash");
        assert_eq!(x.path(), "/no-leading-slash");
        assert_eq!(x.to_string(), "safe://domain/no-leading-slash?v=1");
        x.set_path("");
        assert_eq!(x.path(), ""); // no slash if path is empty.
        assert_eq!(x.to_string(), "safe://domain?v=1");
        x.set_path("/");
        assert_eq!(x.path(), ""); // slash removed if path otherwise empty.
        assert_eq!(x.to_string(), "safe://domain?v=1");

        // url::Url preserves the missing slash, and allows path to
        // merge with domain.  seems kind of broken.  bug?
        u.set_path("no-leading-slash");
        assert_eq!(u.path(), "no-leading-slash");
        assert_eq!(u.to_string(), "safe://domainno-leading-slash?v=1");
        u.set_path("");
        assert_eq!(u.path(), "");
        assert_eq!(x.to_string(), "safe://domain?v=1");
        u.set_path("/");
        assert_eq!(u.path(), "/");
        assert_eq!(x.to_string(), "safe://domain?v=1"); // note that slash in path omitted.

        Ok(())
    }

    #[test]
    fn test_safeurl_to_string() -> Result<()> {
        // These two are equivalent.  ie, the xorurl is the result of nrs.to_xorurl_string()
        let nrsurl = "safe://my.sub.domain/path/my%20dir/my%20file.txt?this=that&this=other&color=blue&v=5&name=John+Doe#somefragment";
        let xorurl = "safe://my.sub.hnyydyiixsfrqix9aoqg97jebuzc6748uc8rykhdd5hjrtg5o4xso9jmggbqh/path/my%20dir/my%20file.txt?this=that&this=other&color=blue&v=5&name=John+Doe#somefragment";

        let nrs = SafeUrl::from_url(nrsurl)?;
        let xor = SafeUrl::from_url(xorurl)?;

        assert_eq!(nrs.to_string(), nrsurl);
        assert_eq!(xor.to_string(), xorurl);

        assert_eq!(nrs.to_nrsurl_string(), Some(nrsurl.to_string()));
        assert_eq!(nrs.to_xorurl_string(), xorurl);

        assert_eq!(xor.to_nrsurl_string(), None);
        assert_eq!(xor.to_xorurl_string(), xorurl);

        Ok(())
    }

    #[test]
    fn test_safeurl_parts() -> Result<()> {
        // These two are equivalent.  ie, the xorurl is the result of nrs.to_xorurl_string()
        let nrsurl = "safe://my.sub.domain/path/my%20dir/my%20file.txt?this=that&this=other&color=blue&v=5&name=John+Doe#somefragment";
        let xorurl = "safe://my.sub.hnyydyiixsfrqix9aoqg97jebuzc6748uc8rykhdd5hjrtg5o4xso9jmggbqh/path/my%20dir/my%20file.txt?this=that&this=other&color=blue&v=5&name=John+Doe#somefragment";

        let nrs = SafeUrl::from_url(nrsurl)?;
        let xor = SafeUrl::from_url(xorurl)?;

        assert_eq!(nrs.scheme(), SAFE_URL_SCHEME);
        assert_eq!(xor.scheme(), SAFE_URL_SCHEME);

        assert_eq!(nrs.host(), "my.sub.domain");
        assert_eq!(
            xor.host(),
            "my.sub.hnyydyiixsfrqix9aoqg97jebuzc6748uc8rykhdd5hjrtg5o4xso9jmggbqh"
        );

        assert_eq!(nrs.path(), "/path/my%20dir/my%20file.txt");
        assert_eq!(xor.path(), "/path/my%20dir/my%20file.txt");

        assert_eq!(nrs.path_decoded()?, "/path/my dir/my file.txt");
        assert_eq!(xor.path_decoded()?, "/path/my dir/my file.txt");

        assert_eq!(
            nrs.query_string(),
            "this=that&this=other&color=blue&v=5&name=John+Doe"
        );
        assert_eq!(
            xor.query_string(),
            "this=that&this=other&color=blue&v=5&name=John+Doe"
        );

        assert_eq!(nrs.fragment(), "somefragment");
        assert_eq!(xor.fragment(), "somefragment");

        Ok(())
    }

    #[test]
    fn test_safeurl_from_url_validation() -> Result<()> {
        // Tests basic URL syntax errors that are common to
        // both ::from_xorurl() and ::from_nrsurl()

        let msg = "Expected error";
        let wrong_err = "Wrong error type";
        let should_work = "should work";

        let result = SafeUrl::from_url("withoutscheme").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("relative URL without a base")),
            _ => panic!(wrong_err),
        }

        let result = SafeUrl::from_url("http://badscheme").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("invalid scheme")),
            _ => panic!(wrong_err),
        }

        let result = SafeUrl::from_url("safe:///emptyhost").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("missing host")),
            _ => panic!(wrong_err),
        }

        let result = SafeUrl::from_url("safe://space in host").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("invalid domain character")),
            _ => panic!(wrong_err),
        }

        let result = SafeUrl::from_url("safe://my.sub..name").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("host contains empty subname")),
            _ => panic!(wrong_err),
        }

        let result = SafeUrl::from_url("safe://hostname//").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("path contains empty component")),
            _ => panic!(wrong_err),
        }

        // note: ?? is actually ok in a standard url.  I suppose no harm in allowing for safe
        // see:  https://stackoverflow.com/questions/2924160/is-it-valid-to-have-more-than-one-question-mark-in-a-url
        let _result = SafeUrl::from_url("safe://hostname??foo=bar").expect(should_work);

        // note: ## and #frag1#frag2 are accepted by rust URL parser.
        // tbd: if we want to disallow.
        // see: https://stackoverflow.com/questions/10850781/multiple-hash-signs-in-url
        let _result = SafeUrl::from_url("safe://hostname?foo=bar##fragment").expect(should_work);

        // note: single%percent/in/path is accepted by rust URL parser.
        // tbd: if we want to disallow.
        let _result =
            SafeUrl::from_nrsurl("safe://hostname/single%percent/in/path").expect(should_work);

        Ok(())
    }

    #[test]
    fn test_safeurl_from_xorurl_validation() -> Result<()> {
        // Tests some URL errors that are specific to xorurl

        let msg = "Expected error";
        let wrong_err = "Wrong error type";

        // test: "Failed to decode XOR-URL"
        let result = SafeUrl::from_xorurl("safe://invalidxor").expect_err(msg);
        match result {
            Error::InvalidXorUrl(e) => assert!(e.contains("Failed to decode XOR-URL")),
            _ => panic!(wrong_err),
        }

        // note: too long/short have separate tests already.
        // "Invalid XOR-URL, encoded string too short"
        // "Invalid XOR-URL, encoded string too long"

        // todo: we should have tests for these.  help anyone?
        // "Invalid or unsupported XOR-URL encoding version: {}",
        // "Invalid content type encoded in the XOR-URL string: {}",
        // "Invalid SAFE data type encoded in the XOR-URL string: {}",

        Ok(())
    }
}
