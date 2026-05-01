#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::delay_ms;
use display_interface_spi::SPIInterface;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use st7789::ST7789;

use avr_device::interrupt;
use panic_halt as _;

use core::cell::Cell;

// Data Type Used to Store the ST7789 Screen Dimensions
struct RealScreenDimensions {
    width: u32,
    height: u32,
}

// Real Screen Dimensions of the ST7789
const REAL_SCREEN_DIMENSIONS: RealScreenDimensions = RealScreenDimensions {
    height: 200,
    width: 280,
};

// Data Type Used to Store the Audio Band Amplitudes
type AudioBandAmplitudes = [u16; 7];

// Due to Skewed Band Responses, They Need to be Normalized
const MAX_MSGEQ7_BAND_RESPONSES: AudioBandAmplitudes = [200, 400, 300, 310, 475, 610, 740];
const MIN_MSGEQ7_BAND_RESPONSES: AudioBandAmplitudes = [100, 200, 200, 210, 110, 150, 400];

// Normalizes the Band Responses
fn normalize_band_response(mut bands: AudioBandAmplitudes) -> AudioBandAmplitudes {
    for index in 0..6 {
        // Normalizing consists of mapping the value to the range [0, 1024]. This will also clamp the values
        let amplitude = bands[index];
        let max = MAX_MSGEQ7_BAND_RESPONSES[index];
        let min = MIN_MSGEQ7_BAND_RESPONSES[index];

        if amplitude > max {
            bands[index] = 1024;
        } else if amplitude < min {
            bands[index] = 0;
        } else {
            let range = (max - min) as u32;
            let offset = (amplitude - min) as u32;

            // x << 10 = x * 1024
            bands[index] = ((offset << 10) / range) as u16;
        }
    }

    bands
}

// Global Variable Used to Keep Track of Milliseconds
static MILLISECOND_COUNTER: interrupt::Mutex<Cell<u32>> = interrupt::Mutex::new(Cell::new(0));

// Executes on 8 bit timer overflow for the timer TC0
#[interrupt(atmega328p)]
fn TIMER0_OVF() {
    // Interrupt the Main Loop
    interrupt::free(|cs| {
        // Update the Millisecond Counter
        let counter = MILLISECOND_COUNTER.borrow(cs);
        let next_val = counter.get().wrapping_add(1);

        counter.set(next_val);
    });
}

// Reads the Millisecond Counter
fn millis() -> u32 {
    interrupt::free(|cs| MILLISECOND_COUNTER.borrow(cs).get())
}

// Part of the Finite State Machine for the MSGEQ7 Reader
#[derive(Copy, Clone)]
enum MSGEQ7ResetState {
    Low { time_set_low_ms: u32 },
    High { time_set_high_ms: u32 },
}

// Timing Constants for resetting the MSGEQ7
impl MSGEQ7ResetState {
    const SET_LOW_WAIT_MS: u32 = 1;
    const SET_HIGH_WAIT_MS: u32 = 1;
}

// Part of the Finite State Machine for the MSGEQ7 Reader
#[derive(Copy, Clone)]
enum MSGEQ7StrobeState {
    Low { time_set_low_ms: u32 },
    High { time_set_high_ms: u32 },
}

// Timing Constants for strobing the MSGEQ7 to read band amplitudes
impl MSGEQ7StrobeState {
    const SET_LOW_WAIT_MS: u32 = 1;
    const SET_HIGH_WAIT_MS: u32 = 1;
}

// Finite State Machine for the MSGEQ7 Reader
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
    // Device peripherals (dp). These are the on-board peripherals.
    let dp = arduino_hal::Peripherals::take().unwrap();

    // Arduino Uno Pins
    let pins = arduino_hal::pins!(dp);

    // Serial Interface (Used to communicate with computer)
    let mut serial = arduino_hal::default_serial!(dp, pins, 57600);

    // Delay object. Allows libraries outside of the main function to use delay
    let mut delay = arduino_hal::Delay::new();

    // Initialize pins needed for the display
    ufmt::uwriteln!(&mut serial, "Initializing pins..\n\n").unwrap();
    let spi_clock = pins.d13.into_output();
    let miso = pins.d12.into_pull_up_input();
    let mosi = pins.d11.into_output();
    let tft_chip_select = pins.d10.into_output();
    let rst = pins.d9.into_output(); // Reset pin
    let data_command = pins.d8.into_output(); // Data/Command pin
    let mut sd_chip_select = pins.d7.into_output();
    let backlight = pins.d6.into_output(); // Backlight pin

    // Disable the SD card by setting its chip select pin high
    sd_chip_select.set_high();

    // Initialize the SPI interface for the display
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

    // Initialize the display SPI interface
    ufmt::uwriteln!(&mut serial, "Initializing display..\n\n").unwrap();
    let display_interface = SPIInterface::new(spi, data_command, spi_cs);

    // Initialize the display interface
    let mut display = ST7789::new(
        display_interface,
        core::prelude::v1::Some(rst),
        core::prelude::v1::Some(backlight),
        300,
        500,
    );

    // Perform initial actions on the display
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

    // Initialize the analog-to-digital converter peripheral
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

    // Initialize the pins used for the msgeq7
    let _msgeq7_clock = pins.d3.into_output(); // Already driven by TC2 hardware
    let mut strobe = pins.d2.into_output();
    let mut reset = pins.d4.into_output();

    // Initialize the pins used for reading the MSGEQ7 output
    let measure = pins.a0.into_analog_input(&mut adc);

    // Initialize the variable which stores the audio band amplitudes
    let mut audio_band_amplitudes: AudioBandAmplitudes = AudioBandAmplitudes::default();

    // Initially reset the chip
    reset.set_low();

    // Initialize the msgeq7 reader
    let mut msgeq7_reader_state = MSGEQ7ReaderState::Resetting(MSGEQ7ResetState::Low {
        time_set_low_ms: millis(),
    });

    // Communicate to the user that the arduino is initialized
    ufmt::uwriteln!(&mut serial, "Arduino Initialized\n").unwrap();

    // Enable interrupts.
    unsafe { interrupt::enable() };

    // Initialize colors
    let bg_color = Rgb565::BLACK;
    let bar_color = Rgb565::CYAN;

    // Variables used for drawing the bands
    let mut most_recent_drawn_bands = AudioBandAmplitudes::default();
    let num_bands = 7;
    let gap = 4; // pixels between bars
    let total_gaps_width = (num_bands + 1) * gap;
    let bar_width = (REAL_SCREEN_DIMENSIONS.width - total_gaps_width) / num_bands;

    display.set_orientation(st7789::Orientation::Landscape).unwrap();

    display.set_pixels(
        0,
        0,
        REAL_SCREEN_DIMENSIONS.width as u16 - 1,
        REAL_SCREEN_DIMENSIONS.height as u16 - 1,
        (0..(REAL_SCREEN_DIMENSIONS.width * REAL_SCREEN_DIMENSIONS.height))
            .map(|_| bg_color.into_storage()),
    )
    .unwrap();

    // Function used to draw the bands
    let mut draw_bands = |new_bands: AudioBandAmplitudes| {
        for index in 0..num_bands as usize {
            let x_start = (gap + (index as u32 * (bar_width + gap))) as u16;

            // Scale the 0-1024 value to the screen height (180)
            // Formula: (value * screen_height) / max_value
            let new_height =
                ((new_bands[index] as u32 * REAL_SCREEN_DIMENSIONS.height) / 1024) as u16;
            let old_height = ((most_recent_drawn_bands[index] as u32
                * REAL_SCREEN_DIMENSIONS.height)
                / 1024) as u16;

            if new_height > old_height {
                // Bar grew: Draw the new segment in CYAN
                let segment_height = new_height - old_height;
                let colors =
                    (0..(bar_width * segment_height as u32)).map(|_| bar_color.into_storage());

                // Note: y=0 is top, so height is measured from bottom (REAL_SCREEN_DIMENSIONS.height)
                display
                    .set_pixels(
                        x_start,
                        REAL_SCREEN_DIMENSIONS.height as u16 - new_height,
                        x_start + bar_width as u16 - 1,
                        (REAL_SCREEN_DIMENSIONS.height as u16 - old_height) - 1,
                        colors,
                    )
                    .unwrap();
            } else if new_height < old_height {
                // Bar shrunk: Erase the top segment with BLACK
                let segment_height = old_height - new_height;
                let colors =
                    (0..(bar_width * segment_height as u32)).map(|_| bg_color.into_storage());

                display
                    .set_pixels(
                        x_start,
                        REAL_SCREEN_DIMENSIONS.height as u16 - old_height,
                        x_start + bar_width as u16 - 1,
                        (REAL_SCREEN_DIMENSIONS.height as u16 - new_height) - 1,
                        colors,
                    )
                    .unwrap();
            }
        }

        // Update the state for the next frame
        most_recent_drawn_bands = new_bands;
    };

    // Main Program Loop
    loop {
        // Get the current monotonic clock millisecond time
        let monotonic_ms = millis();

        // Finite State Machine Operation
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

                            arduino_hal::delay_us(40);
                            // arduino_hal::delay_ms(10);

                            // Read the value of the analog input and store it in the
                            // audio_band_amplitudes array
                            let value = measure.analog_read(&mut adc);
                            audio_band_amplitudes[*frequency_band_index as usize] = value;

                            if *frequency_band_index < 6 {
                                strobe.set_low();

                                // Transition to the next frequency band and set the strobe to low
                                msgeq7_reader_state = MSGEQ7ReaderState::Reading {
                                    strobe_state: MSGEQ7StrobeState::Low {
                                        time_set_low_ms: monotonic_ms,
                                    },
                                    frequency_band_index: *frequency_band_index,
                                }
                            } else {
                                // This case occurs when all 7 frequency bands have been read. In this
                                // scenario, display the values to the screen andreset the MSGEQ7 and
                                // read the next 7 frequency bands.
                                msgeq7_reader_state =
                                    MSGEQ7ReaderState::Resetting(MSGEQ7ResetState::Low {
                                        time_set_low_ms: monotonic_ms,
                                    });

                                // Display Read Values

                                // The commented code below will clear the screen after each print.
                                // This can be enabled or disabled by uncommenting the line
                                /*
                                if overwrite {
                                    ufmt::uwrite!(&mut serial, "\x1b[H").unwrap();
                                }
                                */

                                // Print the audio band amplitudes to the computer (for debugging)
                                /*
                                ufmt::uwriteln!(&mut serial, "{:?}", audio_band_amplitudes)
                                    .unwrap();
                                */

                                // Normalize the audio band amplitudes
                                let normalized_bands =
                                    normalize_band_response(audio_band_amplitudes);

                                ufmt::uwriteln!(&mut serial, "U {:?}", audio_band_amplitudes).unwrap();
                                ufmt::uwriteln!(&mut serial, "N {:?}", normalized_bands).unwrap();

                                // Draw the bands to the display
                                draw_bands(normalized_bands);
                            }
                        }
                    }

                    MSGEQ7StrobeState::Low { time_set_low_ms } => {
                        // If 1 ms elapsed
                        if (monotonic_ms - time_set_low_ms) > MSGEQ7StrobeState::SET_LOW_WAIT_MS {
                            strobe.set_high();

                            // Transition to the next frequency band to be read
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
