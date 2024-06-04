// Copyright (C) 2022-2024 Stephane Raux. Distributed under the 0BSD license.

use brightness::Brightness;
use chrono::Local;
use desktus::{ticks, MouseButton};
use futures::{pin_mut, StreamExt};
use futuristic::StreamTools;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use sysinfo::{System, SystemExt};
use tokio_stream::wrappers::WatchStream;

#[tokio::main]
async fn main() {
    let foreground = palette::named::LIGHTGRAY;
    let date_time = ticks(Duration::from_secs(1)).map(|_| {
        let t = Local::now();
        vec![
            desktus::views::DateView::new(t, Message::Ignore, foreground).render(),
            desktus::views::TimeView::new(t, Message::Ignore, foreground).render(),
        ]
    });
    let battery = ticks(Duration::from_secs(20)).map(|_| {
        desktus::sources::battery_state()
            .ok()
            .flatten()
            .map(|b| desktus::views::BatteryView::new(b, Message::Ignore, foreground).render())
    });
    let cpu = ticks(Duration::from_secs(5)).map({
        let mut system = System::new();
        move |_| {
            desktus::views::CpuView::new(
                desktus::sources::cpu_usage(&mut system),
                Message::Ignore,
                foreground,
            )
            .render()
        }
    });
    let memory = ticks(Duration::from_secs(5)).map({
        let mut system = System::new();
        move |_| {
            desktus::views::MemoryView::new(
                desktus::sources::memory_usage(&mut system),
                Message::Ignore,
                foreground,
            )
            .render()
        }
    });
    let disk = ticks(Duration::from_secs(20)).map({
        let mut system = System::new();
        move |_| {
            desktus::sources::disk_usage(&mut system, "/")
                .ok()
                .map(|d| desktus::views::DiskView::new(d, Message::Ignore, foreground).render())
        }
    });
    let (brightness_notif_sender, brightness_notif_receiver) = tokio::sync::watch::channel(());
    let brightness_notif_sender = &brightness_notif_sender;
    let brightness_notif_receiver = WatchStream::new(brightness_notif_receiver);
    let brightness_triggers = futures::stream::select(
        ticks(Duration::from_secs(20)).map(|_| ()),
        brightness_notif_receiver,
    );
    let brightness = brightness_triggers.then(|_| async {
        desktus::sources::brightness()
            .await
            .into_iter()
            .flatten()
            .map(|b| {
                desktus::views::BrightnessView::new(
                    b,
                    |device| Message::Brightness(device.into()),
                    foreground,
                )
                .render()
            })
            .collect::<Vec<_>>()
    });
    let blocks = brightness
        .zip_latest(disk)
        .zip_latest(memory)
        .zip_latest(cpu)
        .zip_latest(battery)
        .zip_latest(date_time)
        .zip(ticks(Duration::from_secs(1)))
        .map(|(x, _)| x)
        .map(
            |(((((brightness, disk), memory), cpu), battery), date_time)| {
                brightness
                    .into_iter()
                    .chain(disk)
                    .chain(Some(memory))
                    .chain(Some(cpu))
                    .chain(battery)
                    .chain(date_time)
                    .collect::<Vec<_>>()
            },
        );
    let output = desktus::serialize_blocks(blocks, true);
    let input = desktus::events::<Message>().for_each(|event| async move {
        match event.message {
            Message::Brightness(device_name) => {
                change_brightness(&device_name, event.button, brightness_notif_sender).await;
            }
            Message::Ignore => {}
        }
    });
    futures::future::join(output, input).await;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Message {
    Ignore,
    Brightness(String),
}

async fn change_brightness(
    device_name: &str,
    button: MouseButton,
    notify: &tokio::sync::watch::Sender<()>,
) {
    let increment = match button {
        MouseButton::Down => false,
        MouseButton::Up => true,
        _ => return,
    };
    if let Ok(_) = change_brightness_impl(device_name, increment).await {
        let _ = notify.send(());
    }
}

async fn change_brightness_impl(device_name: &str, increment: bool) -> Result<(), ()> {
    let devices = brightness::brightness_devices().filter_map(|device| async move {
        let device = device.ok()?;
        let name = device.device_name().await.ok()?;
        (name == device_name).then(|| device)
    });
    pin_mut!(devices);
    let mut device = devices.next().await.ok_or(())?;
    let level = device.get().await.map_err(|_| ())? as i32;
    let level = level + ((increment as i32) * 2 - 1) * 5;
    let level = level.min(100).max(0) as u32;
    device.set(level).await.map_err(|_| ())
}
