#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use avr_device::interrupt;
use panic_halt as _;
use ufmt::uWrite;

use core::cell::Cell;

static MILLISECOND_COUNTER: interrupt::Mutex<Cell<u32>> = interrupt::Mutex::new(Cell::new(0));
type AudioBandAmplitudes = [u16; 7];

enum ChartDisplayOption {
    Array { overwrite: bool },
    FrequencyBarGraph { overwrite: bool }, // If overwrite is true, serial response will include
                                           // "\x1b[H" control symbol
}

const CHART_DISPLAY_OPTION: ChartDisplayOption =
    ChartDisplayOption::FrequencyBarGraph { overwrite: true };

pub struct ChartBuffer {
    pub storage: [u8; 512], // Buffer to hold the string
    pub len: usize,
}

impl ufmt::uWrite for ChartBuffer {
    type Error = core::convert::Infallible;

    fn write_str(&mut self, string: &str) -> Result<(), Self::Error> {
        let bytes = string.as_bytes();
        let space = self.storage.len() - self.len;
        let to_copy = core::cmp::min(bytes.len(), space);

        self.storage[self.len..self.len + to_copy].copy_from_slice(&bytes[..to_copy]);
        self.len += to_copy;

        Ok(())
    }
}

fn render_audio_band_amplitudes(amplitudes: &[u16; 7]) -> ChartBuffer {
    let mut buffer = ChartBuffer {
        storage: [32; 512],
        len: 0,
    };
    let levels: u16 = 10; // 10 rows high

    // 1. Draw the Bars (Top to Bottom)
    for row in (1..=levels).rev() {
        let threshold = row * 102; // Roughly 1024 / 10
        let _ = buffer.write_str("  "); // Left padding

        for &value in amplitudes.iter() {
            if value >= threshold {
                let _ = buffer.write_str("  #  ");
            } else {
                let _ = buffer.write_str("     ");
            }
        }

        let _ = buffer.write_str("\n");
    }

    // 2. Draw X-Axis line
    let _ = buffer.write_str("  +----+----+----+----+----+----+----+\n");

    // 3. Draw Frequencies (Centered under bars)
    // Labels: 63, 160, 400, 1k, 2.5k, 6.2k, 16k
    let _ = buffer.write_str("  63  160  400  1k  2.5k 6.2k 16k\n");

    buffer
}

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
}
