use {
    hap::{
        accessory::{
            air_quality_sensor::AirQualitySensorAccessory, AccessoryCategory, AccessoryInformation,
        },
        characteristic::{
            carbon_dioxide_level::CarbonDioxideLevelCharacteristic,
            voc_density::VocDensityCharacteristic, AsyncCharacteristicCallbacks,
        },
        futures::future::FutureExt,
        server::{IpServer, Server},
        storage::{FileStorage, Storage},
        tokio, Config, MacAddress, Pin,
    },
    lazy_static::lazy_static,
    linux_embedded_hal::{Delay, I2cdev},
    sgp30::Sgp30,
    std::{
        cmp,
        net::{IpAddr, SocketAddr},
        sync::{Arc, Mutex},
    },
};

const DATA_PATH: &'static str = "/data/hap";

lazy_static! {
    static ref SGP30: Arc<Mutex<Sgp30<I2cdev, Delay>>> = {
        let dev = I2cdev::new("/dev/i2c-1").unwrap();
        let address = 0x58;
        let mut sgp = Sgp30::new(dev, address, Delay);

        sgp.init().unwrap();

        Arc::new(Mutex::new(sgp))
    };
}

#[tokio::main]
async fn main() {
    lazy_static::initialize(&SGP30);

    let current_ipv4 = || -> Option<IpAddr> {
        for iface in pnet::datalink::interfaces() {
            for ip_network in iface.ips {
                if ip_network.is_ipv4() {
                    let ip = ip_network.ip();
                    if !ip.is_loopback() {
                        return Some(ip);
                    }
                }
            }
        }
        None
    };

    let mut accessory = AirQualitySensorAccessory::new(
        1,
        AccessoryInformation {
            name: "SGP30".into(),
            ..Default::default()
        },
    )
    .unwrap();

    accessory
        .air_quality_sensor
        .air_quality
        .on_read_async(Some(|| {
            async {
                let measurement = SGP30.lock().unwrap().measure().unwrap();
                let co2 = measurement.co2eq_ppm;
                let voc = measurement.tvoc_ppb;

                let co2_value = if (0..400).contains(&co2) {
                    1
                } else if (400..1000).contains(&co2) {
                    2
                } else if (1000..2000).contains(&co2) {
                    3
                } else if (2000..5000).contains(&co2) {
                    4
                } else {
                    5
                };

                let voc_value = if (0..25).contains(&voc) {
                    1
                } else if (25..50).contains(&voc) {
                    2
                } else if (50..325).contains(&voc) {
                    3
                } else if (325..500).contains(&voc) {
                    4
                } else {
                    5
                };

                Some(cmp::max(co2_value, voc_value))
            }
            .boxed()
        }));

    accessory.air_quality_sensor.carbon_dioxide_level = {
        let mut characteristic = CarbonDioxideLevelCharacteristic::new(1000, 1);
        characteristic.on_read_async(Some(|| {
            async { Some(SGP30.lock().unwrap().measure().unwrap().co2eq_ppm as f32) }.boxed()
        }));
        Some(characteristic)
    };

    accessory.air_quality_sensor.voc_density = {
        let mut characteristic = VocDensityCharacteristic::new(1001, 1);
        characteristic.on_read_async(Some(|| {
            async { Some(SGP30.lock().unwrap().measure().unwrap().tvoc_ppb as f32) }.boxed()
        }));
        Some(characteristic)
    };

    let mut storage = FileStorage::new(DATA_PATH).await.unwrap();

    let config = match storage.load_config().await {
        Ok(config) => config,
        Err(_) => {
            let config = Config {
                socket_addr: SocketAddr::new(current_ipv4().unwrap(), 32000),
                pin: Pin::new([1, 1, 1, 2, 2, 3, 3, 3]).unwrap(),
                name: "Air Quality Sensor".into(),
                device_id: MacAddress::new([10, 20, 30, 40, 50, 60]),
                category: AccessoryCategory::Sensor,
                ..Default::default()
            };
            storage.save_config(&config).await.unwrap();
            config
        }
    };

    let mut server = IpServer::new(config, storage).unwrap();
    server.add_accessory(accessory).await.unwrap();

    let handle = server.run_handle();

    std::env::set_var("RUST_LOG", "hap=info");
    env_logger::init();

    handle.await;
}
