//! Top level connection object that hold a http client (either synchronous or
//! asynchronous), arango URL, and buffered accessible databases object.
//!
//! ## Establishing connections
//! There is three way to establish connections:
//! - jwt
//! - basic auth
//! - no authentication
//!
//! So are the `arangors` API:
//! Example:
//!
//! - With authentication
//!
//! ```rust
//! use arangors::Connection;
//!
//! # #[cfg_attr(any(feature="reqwest_async"), maybe_async::maybe_async, tokio::main)]
//! # #[cfg_attr(feature = "blocking", maybe_async::must_be_sync)]
//! # async fn main() {
//! let conn = Connection::establish_jwt("http://localhost:8529", "username", "password")
//!     .await
//!     .unwrap();
//! let conn = Connection::establish_basic_auth("http://localhost:8529", "username", "password")
//!     .await
//!     .unwrap();
//! # }
//! ```
//!
//! - No authentication
//! ```rust, ignore
//! use arangors::Connection;
//! let conn = Connection::establish_without_auth("http://localhost:8529").unwrap();
//! ```

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use failure::{format_err, Error};
use http::header::{HeaderMap, AUTHORIZATION, SERVER};
use log::{info, trace};
use serde::{Deserialize, Serialize};
use url::Url;

use maybe_async::maybe_async;

use self::{
    auth::Auth,
    role::{Admin, Normal},
};
use super::{database::Database, response::serialize_response};
use crate::{client::ClientExt, response::ArangoResult};

mod auth;

pub mod role {
    #[derive(Debug)]
    pub struct Normal;

    #[derive(Debug)]
    pub struct Admin;
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Permission {
    #[serde(rename = "none")]
    NoAccess,
    #[serde(rename = "ro")]
    ReadOnly,
    #[serde(rename = "rw")]
    ReadWrite,
}

#[derive(Debug, Deserialize)]
pub struct Version {
    pub server: String,
    pub version: String,
    pub license: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDetails {
    pub name: String,
    pub id: String,
    pub path: String,
    pub is_system: bool,
}

#[cfg(any(feature = "reqwest_async", feature = "reqwest_blocking"))]
pub type Connection = GenericConnection<crate::client::reqwest::ReqwestClient>;

/// Connection is the top level API for this crate.
/// It contains a http client, information about auth, arangodb url, and a hash
/// map of the databases Object. The `databases` Hashmap is construct once
/// connections succeed.
#[derive(Debug)]
pub struct GenericConnection<C: ClientExt, S = Normal> {
    session: Arc<C>,
    arango_url: Url,
    username: String,
    state: S,
    pub(crate) phantom: (),
}

impl<S, C: ClientExt> GenericConnection<C, S> {
    /// Validate the server at given arango url
    ///
    /// Cast `failure::Error` if
    /// - Connection failed
    /// - response code is not 200
    /// - no SERVER header in response header
    /// - SERVER header in response header is not `ArangoDB`
    #[maybe_async]
    pub async fn validate_server(&self) -> Result<(), Error> {
        let arango_url = self.arango_url.as_str();
        let client = C::new(None)?;
        let resp = client.get(arango_url.parse().unwrap(), "").await?;
        // HTTP code 200
        if resp.status().is_success() {
            // have `Server` in header
            if let Some(server) = resp.headers().get(SERVER) {
                // value of `Server` is `ArangoDB`
                let server_value = server.to_str().unwrap();
                if server_value.eq_ignore_ascii_case("ArangoDB") {
                    trace!("Validate arangoDB server done.");
                    return Ok(());
                } else {
                    return Err(format_err!("In HTTP header, Server is {}", server_value));
                }
            } else {
                return Err(format_err!("Fail to find Server in HTTP header"));
            }
        } else {
            return Err(format_err!(
                "Fail to connect to server, Status code: {}",
                resp.status()
            ));
        }
    }

    /// Get url for remote arangoDB server.
    pub fn get_url(&self) -> &Url {
        &self.arango_url
    }

    /// Get HTTP session.
    ///
    /// Users can use this method to get a authorized session to access
    /// arbitrary path on arangoDB Server.
    ///
    /// TODO This method should only be public in this crate when all features
    ///     are implemented.
    pub fn get_session(&self) -> Arc<C> {
        Arc::clone(&self.session)
    }

    /// Get database object with name.
    ///
    /// This function look up accessible database in cache hash map,
    /// and return a reference of database if found.
    #[maybe_async]
    pub async fn db(&self, name: &str) -> Result<Database<'_, C>, Error> {
        let dbs = self.accessible_databases().await?;
        if dbs.contains_key(name) {
            Ok(Database::new(&self, name))
        } else {
            Err(format_err!("Cannot access to db: {}", name))
        }
    }

    /// Get a list of accessible database
    /// 1. retrieve the names of all the accessible databases
    /// 1. for each databases, construct a `Database` object and store them in
    /// `self.databases` for later use
    ///
    /// This function uses the API that is used to retrieve a list of
    /// all databases the current user can access.
    #[maybe_async]
    pub async fn accessible_databases(&self) -> Result<HashMap<String, Permission>, Error> {
        let url = self
            .arango_url
            .join(&format!("/_api/user/{}/database", &self.username))
            .unwrap();
        let resp = self.session.get(url, "").await?;
        let result: ArangoResult<_> = serialize_response(resp.text())?;
        Ok(result.inner)
    }
}

impl<C: ClientExt> GenericConnection<C, Normal> {
    /// Establish connection to ArangoDB sever with Auth.
    ///
    /// The connection is establish in the following steps:
    /// 1. validate if it is a arangoDB server at the given base url
    /// 1. set authentication in header
    /// 1. build a http client that holds authentication tokens
    /// 1. construct databases objects for later use
    ///
    /// The most secure way to connect to a arangoDB server is via JWT
    /// token authentication, along with TLS encryption.
    #[maybe_async]
    async fn establish<T: Into<String>>(
        arango_url: T,
        auth: Auth<'_>,
    ) -> Result<GenericConnection<C, Normal>, Error> {
        let mut conn = GenericConnection {
            arango_url: Url::parse(arango_url.into().as_str())?.join("/").unwrap(),
            username: String::new(),
            session: Arc::new(C::new(None)?),
            state: Normal,
            phantom: (),
        };
        conn.validate_server().await?;

        let user: String;
        let authorization = match auth {
            Auth::Basic(cred) => {
                user = String::from(cred.username);

                let token = base64::encode(&format!("{}:{}", cred.username, cred.password));
                Some(format!("Basic {}", token))
            }
            Auth::Jwt(cred) => {
                user = String::from(cred.username);

                let token = conn.jwt_login(cred.username, cred.password).await?;
                Some(format!("Bearer {}", token))
            }
            Auth::None => {
                user = String::from("root");
                None
            }
        };

        let mut headers = HeaderMap::new();
        if let Some(value) = authorization {
            headers.insert(AUTHORIZATION, value.parse().unwrap());
        }

        conn.username = user;
        conn.session = Arc::new(C::new(headers)?);
        info!("Established");
        Ok(conn)
    }

    /// Establish connection to ArangoDB sever without Authentication.
    ///
    /// The target server **MUST DISABLE** authentication for all requests,
    /// which should only used for **test purpose**.
    ///
    /// Disable authentication means all operations are performed by root user.
    ///
    /// Example:
    /// ```rust, ignore
    /// use arangors::Connection;
    ///
    /// let conn = Connection::establish_without_auth("http://localhost:8529").unwrap();
    /// ```
    #[maybe_async]
    pub async fn establish_without_auth<T: Into<String>>(
        arango_url: T,
    ) -> Result<GenericConnection<C, Normal>, Error> {
        trace!("Establish without auth");
        GenericConnection::establish(arango_url.into(), Auth::None).await
    }

    /// Establish connection to ArangoDB sever with basic auth.
    ///
    /// Example:
    /// ```rust
    /// use arangors::Connection;
    ///
    /// # #[cfg_attr(any(feature="reqwest_async"), maybe_async::maybe_async, tokio::main)]
    /// # #[cfg_attr(feature="blocking", maybe_async::must_be_sync)]
    /// # async fn main() {
    /// let conn = Connection::establish_basic_auth("http://localhost:8529", "username", "password")
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    #[maybe_async]
    pub async fn establish_basic_auth(
        arango_url: &str,
        username: &str,
        password: &str,
    ) -> Result<GenericConnection<C, Normal>, Error> {
        trace!("Establish with basic auth");
        GenericConnection::establish(arango_url, Auth::basic(username, password)).await
    }

    /// Establish connection to ArangoDB sever with jwt authentication.
    ///
    /// Prefered way to interact with arangoDB server.
    ///
    /// JWT token expires after 1 month.
    ///
    /// Example:
    ///
    /// ```rust
    /// use arangors::Connection;
    ///
    /// # #[cfg_attr(any(feature="reqwest_async"), maybe_async::maybe_async, tokio::main)]
    /// # #[cfg_attr(feature = "blocking", maybe_async::must_be_sync)]
    /// # async fn main() {
    /// let conn = Connection::establish_jwt("http://localhost:8529", "username", "password")
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    #[maybe_async]
    pub async fn establish_jwt(
        arango_url: &str,
        username: &str,
        password: &str,
    ) -> Result<GenericConnection<C, Normal>, Error> {
        trace!("Establish with jwt");
        GenericConnection::establish(arango_url, Auth::jwt(username, password)).await
    }

    #[maybe_async]
    async fn jwt_login<T: Into<String>>(&self, username: T, password: T) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct JWT {
            pub jwt: String,
        }
        let url = self.arango_url.join("/_open/auth").unwrap();

        let mut map = HashMap::new();
        map.insert("username", username.into());
        map.insert("password", password.into());

        let jwt: JWT = self
            .session
            .post(url, &serde_json::to_string(&map)?)
            .await?
            .json()?;
        Ok(jwt.jwt)
    }

    #[maybe_async]
    pub async fn into_admin(self) -> Result<GenericConnection<C, Admin>, Error> {
        let dbs = self.accessible_databases().await?;
        let db = dbs
            .get("_system")
            .ok_or(format_err!("Do not have read access to _system database"))?;
        match db {
            Permission::ReadWrite => Ok(self.into()),
            _ => Err(format_err!("Do not have write access to _system database")),
        }
    }
}

impl<C: ClientExt> GenericConnection<C, Admin> {
    /// Create a database via HTTP request and add it into `self.databases`.
    ///
    /// If creation fails, an Error is cast. Otherwise, a bool is returned to
    /// indicate whether the database is correctly created.
    ///
    /// # Example
    /// ```rust
    /// use arangors::Connection;
    /// # #[cfg_attr(any(feature="reqwest_async"), maybe_async::maybe_async, tokio::main)]
    /// # #[cfg_attr(feature = "blocking", maybe_async::must_be_sync)]
    /// # async fn main() {
    /// let conn_normal =
    ///     Connection::establish_jwt("http://localhost:8529", "root", "KWNngteTps7XjrNv")
    ///         .await
    ///         .unwrap();
    /// // consume normal connection and convert it into admin connection
    /// let conn_admin = conn_normal.into_admin().await.unwrap();
    /// let result = conn_admin.create_database("new_db").await.unwrap();
    ///
    /// let mut conn_admin = conn_admin;
    /// let result = conn_admin.drop_database("new_db").await.unwrap();
    /// # }
    /// ```
    /// TODO tweak options on creating database
    #[maybe_async]
    pub async fn create_database(&self, name: &str) -> Result<Database<'_, C>, Error> {
        let mut map = HashMap::new();
        map.insert("name", name);
        let url = self.arango_url.join("/_api/database").unwrap();

        let resp = self
            .session
            .post(url, &serde_json::to_string(&map)?)
            .await?;
        let result: ArangoResult<bool> = serialize_response(resp.text())?;
        match result.inner {
            true => self.db(name).await,
            false => Err(format_err!("Fail to create db. Reason: {:?}", resp)),
        }
    }

    /// Drop database with name.
    ///
    /// This method require a `&mut self`, which means no other reference
    /// to this `Connection` object are allowed. This design avoid references
    /// to drop database at compile time.
    #[maybe_async]
    pub async fn drop_database(&mut self, name: &str) -> Result<(), Error> {
        let url_path = format!("/_api/database/{}", name);
        let url = self.arango_url.join(&url_path).unwrap();

        let resp = self.session.delete(url, "").await?;
        let result: ArangoResult<bool> = serialize_response(resp.text())?;
        match result.inner {
            true => Ok(()),
            false => Err(format_err!("Fail to drop db. Reason: {:?}", resp)),
        }
    }

    pub fn into_normal(self) -> GenericConnection<C, Normal> {
        self.into()
    }
}

impl<C: ClientExt> From<GenericConnection<C, Normal>> for GenericConnection<C, Admin> {
    fn from(conn: GenericConnection<C, Normal>) -> GenericConnection<C, Admin> {
        GenericConnection {
            arango_url: conn.arango_url,
            session: conn.session,
            username: conn.username,
            state: Admin,
            phantom: (),
        }
    }
}

impl<C: ClientExt> From<GenericConnection<C, Admin>> for GenericConnection<C, Normal> {
    fn from(conn: GenericConnection<C, Admin>) -> GenericConnection<C, Normal> {
        GenericConnection {
            arango_url: conn.arango_url,
            session: conn.session,
            username: conn.username,
            state: Normal,
            phantom: (),
        }
    }
}
