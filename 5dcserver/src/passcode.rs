use rand::Rng;

pub fn generate_random_passcode_internal() -> u64 {
    rand::thread_rng().gen_range(0..=2985983) // kkkkkk = 2985983
}
