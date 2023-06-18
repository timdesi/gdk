#[macro_use]
extern crate serde_json;

pub mod error;
mod exchange_rates;

use gdk_common::wally::{make_str, read_str};
use serde_json::Value;

use std::ffi::CString;
use std::io::Write;
use std::os::raw::c_char;
use std::str::FromStr;
use std::sync::{Arc, Once};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gdk_common::model::{InitParam, SPVDownloadHeadersParams, SPVVerifyTxParams};

use crate::error::Error;
use gdk_common::exchange_rates::{ExchangeRatesCache, ExchangeRatesCacher};
use gdk_common::log::{self, debug, info, LevelFilter, Metadata, Record};
use gdk_common::session::{JsonError, Session};
use gdk_common::ureq;
use gdk_electrum::{headers, ElectrumSession, NativeNotif};
use serde::Serialize;

pub const GA_OK: i32 = 0;
pub const GA_ERROR: i32 = -1;
pub const GA_NOT_AUTHORIZED: i32 = -5;

pub struct GdkSession {
    pub backend: GdkBackend,
}

pub enum GdkBackend {
    // Rpc(RpcSession),
    Electrum(ElectrumSession),
    Greenlight(GreenlightSession),
}

#[derive(Default)]
pub struct GreenlightSession {
    xr_cache: ExchangeRatesCache,
}

impl ExchangeRatesCacher for GreenlightSession {
    fn xr_cache(&self) -> ExchangeRatesCache {
        Arc::clone(&self.xr_cache)
    }
}

impl Session for GreenlightSession {
    fn new(_network_parameters: gdk_common::NetworkParameters) -> Result<Self, JsonError> {
        todo!()
    }

    fn native_notification(&mut self) -> &mut NativeNotif {
        todo!()
    }

    fn network_parameters(&self) -> &gdk_common::NetworkParameters {
        todo!()
    }

    fn build_request_agent(&self) -> Result<ureq::Agent, ureq::Error> {
        todo!()
    }

    fn handle_call(&mut self, method: &str, _input: Value) -> Result<Value, JsonError> {
        Err(Error::GreenlightMethodNotFound(method.to_string()).into())
    }
}

impl From<Error> for JsonError {
    fn from(e: Error) -> Self {
        JsonError {
            message: e.to_string(),
            error: e.to_gdk_code(),
        }
    }
}

//
// Session & account management
//

static INIT_LOGGER: Once = Once::new();

#[no_mangle]
pub extern "C" fn GDKRUST_create_session(
    ret: *mut *const libc::c_void,
    network: *const c_char,
) -> i32 {
    let network: Value = match serde_json::from_str(&read_str(network)) {
        Ok(x) => x,
        Err(err) => {
            log::error!("error: {:?}", err);
            return GA_ERROR;
        }
    };

    match create_session(&network) {
        Err(err) => {
            log::error!("create_session error: {}", err);
            GA_ERROR
        }
        Ok(session) => {
            let session = Box::new(session);
            unsafe {
                *ret = Box::into_raw(session) as *mut libc::c_void;
            };
            GA_OK
        }
    }
}

/// Initialize the logging framework.
/// Note that once initialized it cannot be changed, only by reloading the library.
fn init_logging(level: LevelFilter) {
    #[cfg(target_os = "android")]
    INIT_LOGGER.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_min_level(level.to_level().unwrap_or(log::Level::Error))
                .with_filter(
                    android_logger::FilterBuilder::new()
                        .parse("warn,gdk_rust=debug,gdk_electrum=debug")
                        .build(),
                ),
        )
    });

    #[cfg(not(target_os = "android"))]
    INIT_LOGGER.call_once(|| {
        log::set_logger(&LOGGER)
            .map(|()| log::set_max_level(level))
            .expect("cannot initialize logging");
    });
}

fn create_session(network: &Value) -> Result<GdkSession, Value> {
    info!("create_session {:?}", network);
    if !network.is_object() || !network.as_object().unwrap().contains_key("server_type") {
        log::error!("Expected network to be an object with a server_type key");
        return Err(GA_ERROR.into());
    }

    let parsed_network = serde_json::from_value(network.clone());
    if let Err(msg) = parsed_network {
        log::error!("Error parsing network {}", msg);
        return Err(GA_ERROR.into());
    }

    let parsed_network = parsed_network.unwrap();

    let backend = match network["server_type"].as_str() {
        // Some("rpc") => GDKRUST_session::Rpc( GDKRPC_session::create_session(parsed_network.unwrap()).unwrap() ),
        Some("greenlight") => GdkBackend::Greenlight(GreenlightSession::default()),
        Some("electrum") => {
            let session = ElectrumSession::new(parsed_network)?;
            GdkBackend::Electrum(session)
        }
        _ => return Err(json!("server_type invalid")),
    };
    let gdk_session = GdkSession {
        backend,
    };
    Ok(gdk_session)
}

#[no_mangle]
pub extern "C" fn GDKRUST_call_session(
    ptr: *mut libc::c_void,
    method: *const c_char,
    input: *const c_char,
    output: *mut *const c_char,
) -> i32 {
    if ptr.is_null() {
        return GA_ERROR;
    }
    let sess: &mut GdkSession = unsafe { &mut *(ptr as *mut GdkSession) };
    let method = read_str(method);
    let input = read_str(input);

    match call_session(sess, &method, &input) {
        Ok(value) => {
            unsafe { *output = make_str(value.to_string()) };
            GA_OK
        }

        Err(err) => {
            log::error!("error: {:?}", err);

            let retv = if "id_invalid_pin" == err.error {
                GA_NOT_AUTHORIZED
            } else {
                GA_ERROR
            };

            unsafe { *output = make_str(to_string(&err)) };
            retv
        }
    }
}

fn call_session(sess: &mut GdkSession, method: &str, input: &str) -> Result<Value, JsonError> {
    let input = serde_json::from_str(input)?;

    if method == "exchange_rates" {
        let params = serde_json::from_value(input)?;

        let ticker = match sess.backend {
            GdkBackend::Electrum(ref mut s) => exchange_rates::fetch_cached(s, &params),
            GdkBackend::Greenlight(ref mut s) => exchange_rates::fetch_cached(s, &params),
        }?;

        let rate = ticker.map(|t| format!("{:.8}", t.rate)).unwrap_or_default();

        return Ok(json!({ "currencies": { params.currency.to_string(): rate } }));
    }

    // Redact inputs containing private data
    let methods_to_redact_in = vec![
        "login",
        "register_user",
        "encrypt_with_pin",
        "decrypt_with_pin",
        "create_subaccount",
        "credentials_from_pin_data",
        "set_master_blinding_key",
    ];
    let input_str = format!("{:?}", &input);
    let input_redacted = if methods_to_redact_in.contains(&method)
        || input_str.contains("pin")
        || input_str.contains("mnemonic")
        || input_str.contains("xprv")
    {
        "redacted".to_string()
    } else {
        input_str
    };

    info!("GDKRUST_call_session handle_call {} input {:?}", method, input_redacted);

    let res = match sess.backend {
        GdkBackend::Electrum(ref mut s) => s.handle_call(&method, input),
        GdkBackend::Greenlight(ref mut s) => s.handle_call(&method, input),
    };

    let methods_to_redact_out =
        vec!["credentials_from_pin_data", "decrypt_with_pin", "get_master_blinding_key"];
    let mut output_redacted = if methods_to_redact_out.contains(&method) {
        "redacted".to_string()
    } else {
        format!("{:?}", res)
    };
    output_redacted.truncate(200);
    info!("GDKRUST_call_session {} output {:?}", method, output_redacted);

    res
}

#[no_mangle]
pub extern "C" fn GDKRUST_set_notification_handler(
    ptr: *mut libc::c_void,
    handler: extern "C" fn(*const libc::c_void, *const c_char),
    self_context: *const libc::c_void,
) -> i32 {
    if ptr.is_null() {
        return GA_ERROR;
    }
    let sess: &mut GdkSession = unsafe { &mut *(ptr as *mut GdkSession) };
    let backend = &mut sess.backend;

    match backend {
        GdkBackend::Electrum(ref mut s) => s.notify.set_native((handler, self_context)),
        GdkBackend::Greenlight(ref mut _s) => (), // TODO,
    };

    info!("set notification handler");

    GA_OK
}

fn fetch_exchange_rates(agent: ureq::Agent) -> Vec<Ticker> {
    if let Ok(result) = agent.get("https://api-pub.bitfinex.com/v2/tickers?symbols=tBTCUSD").call()
    {
        if let Ok(Value::Array(array)) = result.into_json() {
            if let Some(Value::Array(array)) = array.get(0) {
                // using BIDPRICE https://docs.bitfinex.com/reference#rest-public-tickers
                if let Some(rate) = array.get(1).and_then(|e| e.as_f64()) {
                    let pair = Pair::new(Currency::BTC, Currency::USD);
                    let ticker = Ticker {
                        pair,
                        rate,
                    };
                    info!("got exchange rate {:?}", ticker);
                    return vec![ticker];
                }
            }
        }
    }
    vec![]
}

fn tickers_to_json(tickers: Vec<Ticker>) -> Value {
    let empty_map = serde_json::map::Map::new();
    let currency_map = Value::Object(tickers.iter().fold(empty_map, |mut acc, ticker| {
        let currency = ticker.pair.second();
        acc.insert(currency.to_string(), format!("{:.8}", ticker.rate).into());
        acc
    }));

    json!({ "currencies": currency_map })
}

fn handle_gl_call(
    _session: &mut GreenlightSession,
    method: &str,
    _input: Value,
) -> Result<Value, Error> {
    match method {
        _ => Err(Error::GreenlightMethodNotFound(method.to_string())),
    }
}

// dynamic dispatch shenanigans
fn handle_session_call(
    session: &mut ElectrumSession,
    method: &str,
    input: Value,
) -> Result<Value, Error> {
    match method {
        "poll_session" => session.poll_session().map(|v| json!(v)).map_err(Into::into),

        "connect" => session.connect(&input).map(|v| json!(v)).map_err(Into::into),

        "disconnect" => session.disconnect().map(|v| json!(v)).map_err(Into::into),

        "login" => {
            session.login(serde_json::from_value(input)?).map(|v| json!(v)).map_err(Into::into)
        }
        "credentials_from_pin_data" => session
            .credentials_from_pin_data(serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "encrypt_with_pin" => session
            .encrypt_with_pin(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),

        "get_block_height" => {
            session.get_block_height().map(|block_height| json!(block_height)).map_err(Into::into)
        }

        "get_subaccount_nums" => {
            session.get_subaccount_nums().map(|v| json!(v)).map_err(Into::into)
        }

        "get_subaccounts" => session.get_subaccounts().map(|v| json!(v)).map_err(Into::into),

        "get_subaccount" => get_subaccount(session, &input),

        "discover_subaccount" => session
            .discover_subaccount(serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "get_subaccount_root_path" => session
            .get_subaccount_root_path(serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "get_subaccount_xpub" => session
            .get_subaccount_xpub(serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "create_subaccount" => {
            let opt: CreateAccountOpt = serde_json::from_value(input)?;
            session.create_subaccount(opt).map(|v| json!(v)).map_err(Into::into)
        }
        "get_next_subaccount" => {
            let opt: GetNextAccountOpt = serde_json::from_value(input)?;
            session
                .get_next_subaccount(opt)
                .map(|next_subaccount| json!(next_subaccount))
                .map_err(Into::into)
        }
        "rename_subaccount" => {
            let opt: RenameAccountOpt = serde_json::from_value(input)?;
            session.rename_subaccount(opt).map(|_| json!(true)).map_err(Into::into)
        }
        "set_subaccount_hidden" => {
            let opt: SetAccountHiddenOpt = serde_json::from_value(input)?;
            session.set_subaccount_hidden(opt).map(|_| json!(true)).map_err(Into::into)
        }
        "update_subaccount" => {
            let opt: UpdateAccountOpt = serde_json::from_value(input)?;
            session.update_subaccount(opt).map(|_| json!(true)).map_err(Into::into)
        }

        "get_transactions" => {
            let opt: GetTransactionsOpt = serde_json::from_value(input)?;
            session.get_transactions(&opt).map(|x| txs_result_value(&x)).map_err(Into::into)
        }

        "get_transaction_hex" => {
            get_transaction_hex(session, &input).map(|v| json!(v)).map_err(Into::into)
        }
        "get_transaction_details" => session
            .get_transaction_details(input.as_str().ok_or_else(|| {
                Error::Other("get_transaction_details: input is not a string".into())
            })?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "get_balance" => session
            .get_balance(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "set_transaction_memo" => set_transaction_memo(session, &input),
        "create_transaction" => serialize::create_transaction(session, input),
        "sign_transaction" => session
            .sign_transaction(&serde_json::from_value(input)?)
            .map_err(Into::into)
            .map(|v| json!(v)),
        "send_transaction" => session
            .send_transaction(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "broadcast_transaction" => {
            session
                .broadcast_transaction(input.as_str().ok_or_else(|| {
                    Error::Other("broadcast_transaction: input not a string".into())
                })?)
                .map(|v| json!(v))
                .map_err(Into::into)
        }

        "create_pset" => session
            .create_pset(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "sign_pset" => {
            session.sign_pset(&serde_json::from_value(input)?).map(|v| json!(v)).map_err(Into::into)
        }

        "get_receive_address" => {
            let a = session
                .get_receive_address(&serde_json::from_value(input)?)
                .map(|x| serde_json::to_value(&x).unwrap())
                .map_err(Into::into);
            info!("gdk_rust get_receive_address returning {:?}", a);
            a
        }
        "get_previous_addresses" => session
            .get_previous_addresses(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),

        "get_fee_estimates" => {
            session.get_fee_estimates().map_err(Into::into).and_then(|x| fee_estimate_values(&x))
        }

        "get_settings" => session.get_settings().map_err(Into::into).map(|s| json!(s)),
        "get_available_currencies" => session.get_available_currencies().map_err(Into::into),
        "change_settings" => session
            .change_settings(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),

        "get_unspent_outputs" => session
            .get_unspent_outputs(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "load_store" => session
            .load_store(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "get_master_blinding_key" => {
            session.get_master_blinding_key().map_err(Into::into).map(|s| json!(s))
        }
        "set_master_blinding_key" => session
            .set_master_blinding_key(&serde_json::from_value(input)?)
            .map(|v| json!(v))
            .map_err(Into::into),
        "start_threads" => session.start_threads().map_err(Into::into).map(|s| json!(s)),
        "get_wallet_hash_id" => session.get_wallet_hash_id().map_err(Into::into).map(|s| json!(s)),

        "remove_account" => session.remove_account().map_err(Into::into).map(|s| json!(s)),

        // "auth_handler_get_status" => Ok(auth_handler.to_json()),
        _ => Err(Error::MethodNotFound {
            method: method.to_string(),
            in_session: true,
        }),
    }
}

#[no_mangle]
pub extern "C" fn GDKRUST_destroy_string(ptr: *mut c_char) {
    unsafe {
        // retake pointer and drop
        let _ = CString::from_raw(ptr);
    }
}

#[no_mangle]
pub extern "C" fn GDKRUST_destroy_session(ptr: *mut libc::c_void) {
    unsafe {
        // retake pointer and drop
        let _ = Box::from_raw(ptr as *mut GdkSession);
    }
}

fn build_error(_method: &str, error: &Error) -> String {
    let message = error.to_string();
    let error = error.to_gdk_code();
    let json_error = JsonError {
        message,
        error,
    };
    to_string(&json_error)
}

fn to_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(&value)
        .expect("Default Serialize impl with maps containing only string keys")
}

#[no_mangle]
pub extern "C" fn GDKRUST_call(
    method: *const c_char,
    input: *const c_char,
    output: *mut *const c_char,
) -> i32 {
    let method = read_str(method);
    let input = read_str(input);
    debug!("GDKRUST_call {}", &method);

    let (error_value, result) = match handle_call(&method, &input) {
        Ok(value) => (GA_OK, value),
        Err(err) => (GA_ERROR, build_error(&method, &err)),
    };

    let result = make_str(result);
    unsafe {
        *output = result;
    }
    error_value
}

fn handle_call(method: &str, input: &str) -> Result<String, Error> {
    let start = Instant::now();

    let res = match method {
        "init" => {
            let param: InitParam = serde_json::from_str(input)?;
            init_logging(LevelFilter::from_str(&param.log_level).unwrap_or(LevelFilter::Off));
            gdk_registry::init(&param.registry_dir)?;
            // TODO: read more initialization params
            to_string(&json!("".to_string()))
        }
        "spv_verify_tx" => {
            let param: SPVVerifyTxParams = serde_json::from_str(input)?;
            to_string(&headers::spv_verify_tx(&param)?.as_i32())
        }
        "spv_download_headers" => {
            let param: SPVDownloadHeadersParams = serde_json::from_str(input)?;
            to_string(&headers::download_headers(&param)?)
        }
        "refresh_assets" => {
            let param: gdk_registry::RefreshAssetsParams = serde_json::from_str(input)?;
            to_string(&gdk_registry::refresh_assets(param)?)
        }
        "get_assets" => {
            let params: gdk_registry::GetAssetsParams = serde_json::from_str(input)?;
            to_string(&gdk_registry::get_assets(params)?)
        }

        _ => {
            return Err(Error::MethodNotFound {
                method: method.to_string(),
                in_session: false,
            })
        }
    };

    info!("`{}` took {:?}", method, start.elapsed());

    Ok(res)
}

#[cfg(not(target_os = "android"))]
static LOGGER: SimpleLogger = SimpleLogger;

pub struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let level = metadata.level();
        if level > log::Level::Debug {
            level <= log::max_level()
        } else {
            level <= log::max_level()
                && !metadata.target().starts_with("rustls")
                && !metadata.target().starts_with("electrum_client")
        }
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let ts = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards");
            let _ = writeln!(
                std::io::stdout(),
                "{:02}.{:03} {} - {}",
                ts.as_secs() % 60,
                ts.subsec_millis(),
                record.level(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}
