//! Live weather on a background thread: IP geolocation (ip-api.com) + the free
//! Open-Meteo forecast API (no key). Pushes a `Weather` through a channel; the
//! dashboard reads the latest value.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::dashboard::Weather;

pub fn start() -> Receiver<Weather> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("winland-weather".into())
        .spawn(move || run(tx))
        .expect("spawn weather thread");
    rx
}

fn run(tx: Sender<Weather>) {
    loop {
        match fetch() {
            Some(w) => {
                log::info!(
                    "weather: {} {}F {} (hi {} / lo {})",
                    w.city,
                    w.temp,
                    w.condition,
                    w.high,
                    w.low
                );
                if tx.send(w).is_err() {
                    return;
                }
            }
            None => log::warn!("weather fetch failed"),
        }
        thread::sleep(Duration::from_secs(900)); // refresh every 15 min
    }
}

fn fetch() -> Option<Weather> {
    // 1. Approximate location from IP.
    let geo = ureq::get("http://ip-api.com/json/?fields=city,lat,lon")
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let geo: serde_json::Value = serde_json::from_str(&geo).ok()?;
    let city = geo.get("city")?.as_str()?.to_string();
    let lat = geo.get("lat")?.as_f64()?;
    let lon = geo.get("lon")?.as_f64()?;

    // 2. Forecast from Open-Meteo (Fahrenheit, to match the reference).
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,weather_code\
         &daily=temperature_2m_max,temperature_2m_min\
         &temperature_unit=fahrenheit&timezone=auto&forecast_days=1"
    );
    let body = ureq::get(&url)
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let j: serde_json::Value = serde_json::from_str(&body).ok()?;

    let temp = j["current"]["temperature_2m"].as_f64()?.round() as i32;
    let code = j["current"]["weather_code"].as_u64()? as u8;
    let high = j["daily"]["temperature_2m_max"][0].as_f64()?.round() as i32;
    let low = j["daily"]["temperature_2m_min"][0].as_f64()?.round() as i32;

    Some(Weather {
        temp,
        condition: condition_str(code).to_string(),
        city,
        high,
        low,
        code,
    })
}

fn condition_str(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mainly Clear",
        2 => "Partly Cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing Drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing Rain",
        71 | 73 | 75 | 77 => "Snow",
        80 | 81 | 82 => "Rain Showers",
        85 | 86 => "Snow Showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, Hail",
        _ => "Weather",
    }
}
