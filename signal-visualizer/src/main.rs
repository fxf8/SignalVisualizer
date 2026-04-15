#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::delay_ms;
use display_interface_spi::SPIInterface;
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
};
use embedded_hal::{delay::DelayNs, digital::OutputPin};
use st7789::{Orientation, ST7789};

use avr_device::interrupt;
use panic_halt as _;
use ufmt::uWrite;

use core::cell::Cell;

struct RealScreenDimensions {
    width: u32,
    height: u32,
}

const REAL_SCREEN_DIMENSIONS: RealScreenDimensions = RealScreenDimensions {
    width: 180,
    height: 270,
};

static MILLISECOND_COUNTER: interrupt::Mutex<Cell<u32>> = interrupt::Mutex::new(Cell::new(0));
type AudioBandAmplitudes = [u16; 7];

#[interrupt(atmega328p)]
fn TIMER0_OVF() {
    // Executes on 8 bit timer overflow
    interrupt::free(|cs| {
        let counter = MILLISECOND_COUNTER.borrow(cs);
        let next_val = counter.get().wrapping_add(1);
        counter.set(next_val);
    });
}

fn millis() -> u32 {
    interrupt::free(|cs| MILLISECOND_COUNTER.borrow(cs).get())
}

#[derive(Copy, Clone)]
enum MSGEQ7ResetState {
    Low { time_set_low_ms: u32 },
    High { time_set_high_ms: u32 },
}

impl MSGEQ7ResetState {
    const SET_LOW_WAIT_MS: u32 = 1;
    const SET_HIGH_WAIT_MS: u32 = 1;
}

#[derive(Copy, Clone)]
enum MSGEQ7StrobeState {
    Low { time_set_low_ms: u32 },
    High { time_set_high_ms: u32 },
}

impl MSGEQ7StrobeState {
    const SET_LOW_WAIT_MS: u32 = 1;
    const SET_HIGH_WAIT_MS: u32 = 1;
}

#[derive(Copy, Clone)]
enum MSGEQ7ReaderState {
    Resetting(MSGEQ7ResetState),
    Reading {
        strobe_state: MSGEQ7StrobeState,
        frequency_band_index: u8, // Holds values [0, 6] to account for 7 frequency bands
    },
}

#[arduino_hal::entry]
fn main() -> ! {
    // Device peripherals
    let dp = arduino_hal::Peripherals::take().unwrap();

    let pins = arduino_hal::pins!(dp);

    let mut serial = arduino_hal::default_serial!(dp, pins, 57600);
    let mut delay = arduino_hal::Delay::new();

    ufmt::uwriteln!(&mut serial, "Initializing pins..\n\n").unwrap();
    let spi_clock = pins.d13.into_output();
    let miso = pins.d12.into_pull_up_input();
    let mosi = pins.d11.into_output();
    let tft_chip_select = pins.d10.into_output();
    let rst = pins.d9.into_output(); // Reset pin
    let data_command = pins.d8.into_output(); // Data/Command pin
    let mut sd_chip_select = pins.d7.into_output();
    let backlight = pins.d6.into_output(); // Backlight pin

    sd_chip_select.set_high();

    ufmt::uwriteln!(&mut serial, "Initializing SPI..\n\n").unwrap();
    let (spi, spi_cs) = arduino_hal::Spi::new(
        dp.SPI,
        spi_clock,
        mosi, // MOSI
        miso, // MISO (unused)
        tft_chip_select,
        arduino_hal::spi::Settings {
            data_order: arduino_hal::spi::DataOrder::MostSignificantFirst,
            clock: arduino_hal::spi::SerialClockRate::OscfOver2,
            mode: embedded_hal::spi::MODE_0,
        },
    );

    ufmt::uwriteln!(&mut serial, "Initializing display..\n\n").unwrap();
    let display_interface = SPIInterface::new(spi, data_command, spi_cs);

    let mut display = ST7789::new(
        display_interface,
        core::prelude::v1::Some(rst),
        core::prelude::v1::Some(backlight),
        300,
        500,
    );

    ufmt::uwriteln!(&mut serial, "Configuring display..\n\n").unwrap();
    display.init(&mut delay).unwrap();
    display
        .set_backlight(st7789::BacklightState::On, &mut delay)
        .unwrap();

    ufmt::uwriteln!(&mut serial, "Resetting Display..\n\n").unwrap();
    display.hard_reset(&mut delay).unwrap();
    delay_ms(200);
    ufmt::uwriteln!(&mut serial, "Initializing Display..\n\n").unwrap();
    display.init(&mut delay).unwrap();
    delay_ms(200);

    let mut color_cycle = [
        Rgb565::CSS_ORANGE,
        Rgb565::CSS_GREEN,
        Rgb565::CSS_BLUE,
        Rgb565::CSS_RED,
    ]
    .iter()
    .map(|&c| c.into_storage())
    .cycle();

    loop {
        ufmt::uwriteln!(&mut serial, "Drawing pixels\n").unwrap();

        let color = color_cycle.next().unwrap();

        let x_start = 10;
        let y_start = 10;
        let width = 180;
        let height = 270;
        let colors = (0..(width as u32 * height as u32)).map(|_| color);

        display
            .set_pixels(
                x_start,
                y_start,
                x_start + width - 1,
                y_start + height - 1,
                colors,
            )
            .unwrap();
    }

    /*
    let mut adc = arduino_hal::Adc::new(dp.ADC, Default::default());

    // Used for monotonic clock
    let timer0 = dp.TC0;

    // Used for msgeq7 ckin
    let timer2 = dp.TC2;

    // Monotonic Clock Configuration
    // Timer 0 (Used for asynchrony)
    // Make the 8 bit timer only count every 64 clock cycles
    timer0.tccr0b().write(|w| w.cs0().prescale_64());
    // Enable overflow interrupt
    timer0.timsk0().write(|w| w.toie0().set_bit());

    // Timer 2 (Used for MSGEQ7 Clock Generation)
    timer2
        .tccr2a()
        .write(|w| w.com2b().match_toggle().wgm2().ctc());
    timer2.tccr2b().write(|w| w.cs2().direct());
    timer2.ocr2a().write(|w| unsafe { w.bits(47) });

    let _msgeq7_clock = pins.d3.into_output(); // Already driven by TC2 hardware
    let mut strobe = pins.d2.into_output();
    let mut reset = pins.d4.into_output();

    let measure = pins.a0.into_analog_input(&mut adc);

    let mut audio_band_amplitudes: AudioBandAmplitudes = AudioBandAmplitudes::default();

    // Initially reset the chip
    reset.set_low();

    let mut msgeq7_reader_state = MSGEQ7ReaderState::Resetting(MSGEQ7ResetState::Low {
        time_set_low_ms: millis(),
    });

    ufmt::uwriteln!(&mut serial, "Arduino Initialized\n").unwrap();

    unsafe { interrupt::enable() };

    loop {
        let monotonic_ms = millis();

        match msgeq7_reader_state {
            MSGEQ7ReaderState::Resetting(reset_state) => {
                match reset_state {
                    MSGEQ7ResetState::Low { time_set_low_ms } => {
                        if (monotonic_ms - time_set_low_ms) > MSGEQ7ResetState::SET_LOW_WAIT_MS {
                            reset.set_high();

                            msgeq7_reader_state =
                                MSGEQ7ReaderState::Resetting(MSGEQ7ResetState::High {
                                    time_set_high_ms: monotonic_ms,
                                })
                        }
                    }

                    MSGEQ7ResetState::High { time_set_high_ms } => {
                        // If 20 ms elapsed
                        if (monotonic_ms - time_set_high_ms) > MSGEQ7ResetState::SET_HIGH_WAIT_MS {
                            reset.set_low();
                            strobe.set_high();

                            msgeq7_reader_state = MSGEQ7ReaderState::Reading {
                                strobe_state: MSGEQ7StrobeState::High {
                                    time_set_high_ms: monotonic_ms,
                                },
                                frequency_band_index: 0,
                            }
                        }
                    }
                }
            }

            MSGEQ7ReaderState::Reading {
                ref frequency_band_index,
                ref strobe_state,
            } => {
                match strobe_state {
                    MSGEQ7StrobeState::High { time_set_high_ms } => {
                        // If 20 ms elapsed
                        if (monotonic_ms - time_set_high_ms) > MSGEQ7StrobeState::SET_HIGH_WAIT_MS {
                            strobe.set_low();

                            // arduino_hal::delay_us(40);
                            arduino_hal::delay_ms(10);

                            let value = measure.analog_read(&mut adc);
                            audio_band_amplitudes[*frequency_band_index as usize] = value as u16;

                            if *frequency_band_index < 6 {
                                strobe.set_low();

                                msgeq7_reader_state = MSGEQ7ReaderState::Reading {
                                    strobe_state: MSGEQ7StrobeState::Low {
                                        time_set_low_ms: monotonic_ms,
                                    },
                                    frequency_band_index: *frequency_band_index,
                                }
                            } else {
                                msgeq7_reader_state =
                                    MSGEQ7ReaderState::Resetting(MSGEQ7ResetState::Low {
                                        time_set_low_ms: monotonic_ms,
                                    });

                                // Display Read Values

                                match CHART_DISPLAY_OPTION {
                                    ChartDisplayOption::Array { overwrite } => {
                                        if overwrite {
                                            ufmt::uwrite!(&mut serial, "\x1b[H").unwrap();
                                        }

                                        ufmt::uwriteln!(&mut serial, "{:?}", audio_band_amplitudes)
                                            .unwrap();
                                    }

                                    ChartDisplayOption::FrequencyBarGraph { overwrite } => {
                                        let chart =
                                            render_audio_band_amplitudes(&audio_band_amplitudes);

                                        if overwrite {
                                            ufmt::uwrite!(&mut serial, "\x1b[H").unwrap();
                                        }

                                        if let Ok(s) =
                                            core::str::from_utf8(&chart.storage[..chart.len])
                                        {
                                            ufmt::uwriteln!(&mut serial, "{}", s).unwrap();
                                        }
                                    }
                                }
                            }
                        }
                    }

                    MSGEQ7StrobeState::Low { time_set_low_ms } => {
                        // If 20 ms elapsed
                        if (monotonic_ms - time_set_low_ms) > MSGEQ7StrobeState::SET_LOW_WAIT_MS {
                            strobe.set_high();

                            msgeq7_reader_state = MSGEQ7ReaderState::Reading {
                                strobe_state: MSGEQ7StrobeState::High {
                                    time_set_high_ms: monotonic_ms,
                                },
                                frequency_band_index: *frequency_band_index + 1,
                            }
                        }
                    }
                }
            }
        }
    }
    */
}
