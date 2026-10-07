use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;

use crate::clock::{beijing_now, beijing_offset};

#[derive(Clone, Debug)]
pub struct Bar {
    pub label: String,
    pub close: f64,
}

#[derive(Clone, Debug)]
pub struct Quote {
    pub name: &'static str,
    pub code: String,
    pub price: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub change: f64,
    pub change_pct: f64,
    pub amount_yi: Option<f64>,
    pub time: String,
}

#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub sh: Option<Quote>,
    pub sz: Option<Quote>,
    pub sh_daily: Vec<Bar>,
    pub sh_intra: Vec<Bar>,
    pub btc_usd: Option<f64>,
    pub btc_open: Option<f64>,
    pub btc_high: Option<f64>,
    pub btc_low: Option<f64>,
    pub btc_spark: Vec<Bar>,
    pub usdcny: Option<f64>,
    pub notes: Vec<String>,
    pub fetched_at: String,
}

pub fn fetch_all(client: &Client) -> Feed {
    let mut feed = Feed {
        fetched_at: beijing_now().format("%H:%M:%S").to_string(),
        ..Feed::default()
    };

    let (quotes, daily, intra, btc) = std::thread::scope(|scope| {
        let quotes = scope.spawn(|| fetch_quotes(client));
        let daily = scope.spawn(|| fetch_daily(client));
        let intra = scope.spawn(|| fetch_intraday(client));
        let btc = scope.spawn(|| fetch_btc(client));
        (quotes.join(), daily.join(), intra.join(), btc.join())
    });

    match quotes {
        Ok(Ok((sh, sz, fx))) => {
            if sh.is_none() {
                feed.notes.push("上证指数没有返回".into());
            }
            feed.sh = sh;
            feed.sz = sz;
            feed.usdcny = fx;
        }
        Ok(Err(err)) => feed.notes.push(format!("行情 {err}")),
        Err(_) => feed.notes.push("行情线程中断".into()),
    }
    match daily {
        Ok(Ok(bars)) => feed.sh_daily = bars,
        Ok(Err(err)) => feed.notes.push(format!("日K {err}")),
        Err(_) => feed.notes.push("日K线程中断".into()),
    }
    match intra {
        Ok(Ok(bars)) => feed.sh_intra = bars,
        Ok(Err(err)) => feed.notes.push(format!("分时 {err}")),
        Err(_) => feed.notes.push("分时线程中断".into()),
    }
    match btc {
        Ok(Ok((last, open, high, low, spark))) => {
            feed.btc_usd = Some(last);
            feed.btc_open = Some(open);
            feed.btc_high = Some(high);
            feed.btc_low = Some(low);
            feed.btc_spark = spark;
        }
        Ok(Err(err)) => feed.notes.push(format!("比特币 {err}")),
        Err(_) => feed.notes.push("比特币线程中断".into()),
    }
    feed
}

fn http_client(direct: bool) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) desk-widget");
    if direct {
        // 环境里可能残留一个已经关掉的本地代理。行情源直连可用，不要被它拦住。
        builder = builder.no_proxy();
    }
    builder.build().map_err(|err| err.to_string())
}

fn merge_missing(base: &mut Feed, extra: Feed) {
    if base.sh.is_none() {
        base.sh = extra.sh;
    }
    if base.sz.is_none() {
        base.sz = extra.sz;
    }
    if base.usdcny.is_none() {
        base.usdcny = extra.usdcny;
    }
    if base.sh_daily.is_empty() {
        base.sh_daily = extra.sh_daily;
    }
    if base.sh_intra.is_empty() {
        base.sh_intra = extra.sh_intra;
    }
    if base.btc_usd.is_none() {
        base.btc_usd = extra.btc_usd;
        base.btc_open = extra.btc_open;
        base.btc_high = extra.btc_high;
        base.btc_low = extra.btc_low;
        base.btc_spark = extra.btc_spark;
    }
    if base.sh.is_some() {
        base.notes.retain(|note| !note.starts_with("行情"));
    }
    if !base.sh_daily.is_empty() {
        base.notes.retain(|note| !note.starts_with("日K"));
    }
    if !base.sh_intra.is_empty() {
        base.notes.retain(|note| !note.starts_with("分时"));
    }
    if base.btc_usd.is_some() {
        base.notes.retain(|note| !note.starts_with("比特币"));
    }
}

pub fn spawn_feed() -> (std::sync::mpsc::Receiver<Feed>, std::sync::mpsc::Sender<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let direct = match http_client(true) {
            Ok(client) => client,
            Err(err) => {
                let _ = tx.send(Feed {
                    notes: vec![err],
                    fetched_at: beijing_now().format("%H:%M:%S").to_string(),
                    ..Feed::default()
                });
                return;
            }
        };
        let proxy = http_client(false).ok();
        loop {
            let mut feed = fetch_all(&direct);
            if (feed.sh.is_none() || feed.btc_usd.is_none())
                && let Some(proxy) = &proxy
            {
                merge_missing(&mut feed, fetch_all(proxy));
            }
            let _ = tx.send(feed);
            let _ = cmd_rx.recv_timeout(Duration::from_secs(20));
            while cmd_rx.try_recv().is_ok() {}
        }
    });
    (rx, cmd_tx)
}

fn get_bytes(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    response.bytes().map(|b| b.to_vec()).map_err(|err| err.to_string())
}

fn get_utf8(client: &Client, url: &str) -> Result<String, String> {
    let bytes = get_bytes(client, url)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn fetch_quotes(client: &Client) -> Result<(Option<Quote>, Option<Quote>, Option<f64>), String> {
    let bytes = get_bytes(client, "https://qt.gtimg.cn/q=sh000001,sz399001,fxUSDCNY")?;
    let parsed = parse_quotes_bytes(&bytes);
    if parsed.0.is_none() && parsed.2.is_none() {
        return Err("返回内容无法解析".into());
    }
    Ok(parsed)
}

fn fetch_daily(client: &Client) -> Result<Vec<Bar>, String> {
    let text = get_utf8(
        client,
        "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param=sh000001,day,,,60,qfq",
    )?;
    parse_daily(&text)
}

fn fetch_intraday(client: &Client) -> Result<Vec<Bar>, String> {
    let text = get_utf8(
        client,
        "https://web.ifzq.gtimg.cn/appstock/app/minute/query?code=sh000001",
    )?;
    parse_intraday(&text)
}

fn fetch_btc(client: &Client) -> Result<(f64, f64, f64, f64, Vec<Bar>), String> {
    // 芝麻开门现货接口在国内可直连，币安官方域名从这台机器连不上。
    let ticker = get_utf8(
        client,
        "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=BTC_USDT",
    )?;
    let candles = get_utf8(
        client,
        "https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair=BTC_USDT&interval=15m&limit=96",
    )?;
    parse_btc(&ticker, &candles)
}

#[cfg(test)]
pub fn parse_quotes(body: &str) -> (Option<Quote>, Option<Quote>, Option<f64>) {
    parse_quotes_bytes(body.as_bytes())
}

/// 腾讯行情是 GBK。`~` 的字节 0x7E 也可能是汉字的第二字节，按字符边界切开再取数字字段。
fn parse_quotes_bytes(body: &[u8]) -> (Option<Quote>, Option<Quote>, Option<f64>) {
    let mut sh = None;
    let mut sz = None;
    let mut fx = None;
    for part in split_ascii(body, b';') {
        let part = trim_ascii(part);
        if part.is_empty() {
            continue;
        }
        let Some(eq) = part.iter().position(|b| *b == b'=') else {
            continue;
        };
        let key = String::from_utf8_lossy(&part[..eq]);
        let mut inner = trim_ascii(&part[eq + 1..]);
        if inner.first() == Some(&b'"') {
            inner = &inner[1..];
        }
        if inner.last() == Some(&b'"') {
            inner = &inner[..inner.len() - 1];
        }
        let fields = split_gbk_tilde(inner);
        if key.contains("fxUSDCNY") {
            fx = fields.get(3).and_then(|s| s.parse().ok());
        } else if key.contains("sh000001") {
            sh = quote_from_fields(&fields, "上证指数");
        } else if key.contains("sz399001") {
            sz = quote_from_fields(&fields, "深证成指");
        }
    }
    (sh, sz, fx)
}

fn quote_from_fields(fields: &[String], name: &'static str) -> Option<Quote> {
    if fields.len() < 35 {
        return None;
    }
    let price: f64 = fields[3].parse().ok()?;
    if price <= 0.0 {
        return None;
    }
    let prev: f64 = fields[4].parse().unwrap_or(price);
    let open: f64 = fields[5].parse().unwrap_or(price);
    let change: f64 = fields[31].parse().unwrap_or(price - prev);
    let change_pct: f64 = fields[32].parse().unwrap_or(if prev != 0.0 {
        change / prev * 100.0
    } else {
        0.0
    });
    let high: f64 = fields[33].parse().unwrap_or(price);
    let low: f64 = fields[34].parse().unwrap_or(price);
    let amount_yi = fields
        .get(35)
        .and_then(|s| s.split('/').nth(2))
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .map(|v| v / 1e8);
    Some(Quote {
        name,
        code: fields[2].clone(),
        price,
        open,
        high,
        low,
        change,
        change_pct,
        amount_yi,
        time: fields.get(30).cloned().unwrap_or_default(),
    })
}

fn split_ascii(bytes: &[u8], sep: u8) -> Vec<&[u8]> {
    bytes.split(|b| *b == sep).collect()
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn split_gbk_tilde(bytes: &[u8]) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if (0x81..=0xFE).contains(&b) && i + 1 < bytes.len() {
            let trail = bytes[i + 1];
            if (0x40..=0x7E).contains(&trail) || (0x80..=0xFE).contains(&trail) {
                current.push(b);
                current.push(trail);
                i += 2;
                continue;
            }
        }
        if b == b'~' {
            fields.push(String::from_utf8_lossy(&current).into_owned());
            current.clear();
            i += 1;
            continue;
        }
        current.push(b);
        i += 1;
    }
    fields.push(String::from_utf8_lossy(&current).into_owned());
    fields
}

pub fn parse_daily(text: &str) -> Result<Vec<Bar>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let node = &value["data"]["sh000001"];
    let days = node["day"]
        .as_array()
        .or_else(|| node["qfqday"].as_array())
        .ok_or("日K为空")?;
    let mut bars = Vec::new();
    for row in days {
        let Some(cols) = row.as_array() else { continue };
        if cols.len() < 3 {
            continue;
        }
        let label = cols[0].as_str().unwrap_or("").to_string();
        let Some(close) = json_number(&cols[2]) else { continue };
        let short = if label.len() >= 10 { label[5..].to_string() } else { label };
        bars.push(Bar { label: short, close });
    }
    if bars.is_empty() {
        return Err("日K为空".into());
    }
    Ok(bars)
}

pub fn parse_intraday(text: &str) -> Result<Vec<Bar>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let rows = value["data"]["sh000001"]["data"]["data"]
        .as_array()
        .ok_or("分时为空")?;
    let mut bars = Vec::new();
    for row in rows {
        let Some(text) = row.as_str() else { continue };
        let mut parts = text.split_whitespace();
        let hm = parts.next().unwrap_or("");
        let Some(close) = parts.next().and_then(|s| s.parse().ok()) else { continue };
        let label = if hm.len() == 4 {
            format!("{}:{}", &hm[..2], &hm[2..])
        } else {
            hm.to_string()
        };
        bars.push(Bar { label, close });
    }
    if bars.is_empty() {
        return Err("分时为空".into());
    }
    Ok(bars)
}

pub fn parse_btc(ticker: &str, candles: &str) -> Result<(f64, f64, f64, f64, Vec<Bar>), String> {
    let ticker: serde_json::Value = serde_json::from_str(ticker).map_err(|err| err.to_string())?;
    let row = ticker.as_array().and_then(|rows| rows.first()).ok_or("缺少行情")?;
    let last = json_str_f64(&row["last"]).ok_or("缺少最新价")?;
    let high = json_str_f64(&row["high_24h"]).unwrap_or(last);
    let low = json_str_f64(&row["low_24h"]).unwrap_or(last);
    let pct = json_str_f64(&row["change_percentage"]).unwrap_or(0.0);
    let denom = 1.0 + pct / 100.0;
    let open = if denom.abs() > 1e-9 { last / denom } else { last };

    let candles: serde_json::Value = serde_json::from_str(candles).map_err(|err| err.to_string())?;
    let rows = candles.as_array().ok_or("K 线为空")?;
    let mut spark = Vec::new();
    for item in rows {
        let Some(cols) = item.as_array() else { continue };
        // 芝麻开门：时间(秒)、计价成交额、收、高、低、开
        if cols.len() < 3 {
            continue;
        }
        let Some(close) = json_str_f64(&cols[2]) else { continue };
        if close <= 0.0 {
            continue;
        }
        let ts = json_str_f64(&cols[0]).map(|v| v as i64).unwrap_or(0);
        spark.push((ts, Bar {
            label: candle_label(candle_millis(ts)),
            close,
        }));
    }
    spark.sort_by_key(|(ts, _)| *ts);
    Ok((last, open, high, low, spark.into_iter().map(|(_, bar)| bar).collect()))
}

fn candle_millis(ts: i64) -> i64 {
    if ts > 0 && ts < 1_000_000_000_000 { ts * 1000 } else { ts }
}

fn candle_label(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .map(|dt| dt.with_timezone(&beijing_offset()).format("%H:%M").to_string())
        .unwrap_or_default()
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_f64())
}

fn json_str_f64(value: &serde_json::Value) -> Option<f64> {
    value.as_str().and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要外网"]
    fn live_quotes_parse() {
        let client = http_client(true).expect("client");
        let feed = fetch_all(&client);
        assert!(feed.sh.is_some(), "上证: {:?}", feed.notes);
        assert!(feed.sh_daily.len() >= 2, "日K: {:?}", feed.notes);
        assert!(feed.btc_usd.is_some(), "比特币: {:?}", feed.notes);
        assert!(feed.usdcny.is_some(), "汇率: {:?}", feed.notes);
    }


    #[test]
    fn parses_tencent_quotes() {
        let body = r#"v_sh000001="1~上证指数~000001~3842.19~3830.45~3839.25~1~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~~20260930161500~11.74~0.31~3851.22~3833.09~3842.19/414560247/679398992445~";v_sz399001="1~深证成指~399001~12000.50~11900.00~11950.00~1~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~~20260930161500~100.50~0.84~12100.00~11880.00~12000.50/1/500000000000~";v_fxUSDCNY="310~美元~USDCNY~6.7050~0~";"#;
        let (sh, sz, fx) = parse_quotes(body);
        let sh = sh.expect("sh");
        assert_eq!(sh.price, 3842.19);
        assert_eq!(sh.change, 11.74);
        assert_eq!(sh.change_pct, 0.31);
        assert_eq!(sh.high, 3851.22);
        assert!((sh.amount_yi.unwrap() - 6793.98992445).abs() < 0.01);
        assert_eq!(sz.expect("sz").price, 12000.50);
        assert_eq!(fx, Some(6.7050));
    }

    #[test]
    fn parses_daily_and_intraday() {
        let daily = r#"{"data":{"sh000001":{"day":[["2026-09-29","1","3830.00","2","3","4"],["2026-09-30","1","3842.19","2","3","4"]]}}}"#;
        let bars = parse_daily(daily).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].label, "09-30");
        assert_eq!(bars[1].close, 3842.19);

        let intra = r#"{"data":{"sh000001":{"data":{"data":["0930 3839.25 1 2","0931 3838.14 1 2"]}}}}"#;
        let bars = parse_intraday(intra).unwrap();
        assert_eq!(bars[0].label, "09:30");
        assert_eq!(bars[1].close, 3838.14);
    }

    #[test]
    fn parses_gate_btc_newest_first() {
        let ticker = r#"[{"currency_pair":"BTC_USDT","last":"85814.1","change_percentage":"-0.53","high_24h":"86989.4","low_24h":"84990.1"}]"#;
        let candles = r#"[["1791241200","1","85865.2","85900","85700","85800","1","true"],["1791237600","1","85932.8","86000","85800","85900","1","true"]]"#;
        let (last, open, high, low, spark) = parse_btc(ticker, candles).unwrap();
        assert_eq!(last, 85814.1);
        assert!((open - last / 0.9947).abs() < 0.01);
        assert_eq!(high, 86989.4);
        assert_eq!(low, 84990.1);
        assert_eq!(spark[0].close, 85932.8);
        assert_eq!(spark[1].close, 85865.2);
        assert!(!spark[0].label.is_empty());
    }
}
