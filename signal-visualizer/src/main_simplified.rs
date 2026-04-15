match msgeq7_reader_state {
    MSGEQ7ReaderState::Resetting(reset_state) => {
        match reset_state {
            MSGEQ7ResetState::Low { time_set_low_ms } => {
                //...
            }

            MSGEQ7ResetState::High { time_set_high_ms } => {
                //...
            }
        }
    }

    MSGEQ7ReaderState::Reading {
        ref frequency_band_index,
        ref strobe_state,
    } => {
        match strobe_state {
            MSGEQ7StrobeState::High { time_set_high_ms } => {
                //...
            }

            MSGEQ7StrobeState::Low { time_set_low_ms } => {
                //..
            }
        }
    }
}
