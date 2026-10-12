pub mod floor;
pub mod raffle;
pub mod registry;
pub mod treasury;
pub mod wake;
pub mod wrap;

pub use floor::*;
pub use raffle::*;
pub use registry::*;
pub use treasury::*;
pub use wake::*;
pub use wrap::*;

/// The layout contract with rook_backing (`rook_layouts::{listing, module, module_vault, wrapper, core_seeds}`): rook_backing
/// reads these accounts by those offsets (`register_partner`, `back`), so this test fails if a field it reads moves.
#[cfg(test)]
mod layouts_tests {
    use super::*;
    use anchor_lang::prelude::*;
    use anchor_lang::Discriminator;

    fn zeroed<T: AccountDeserialize + Discriminator>(space: usize) -> T {
        let mut d = vec![0u8; 8 + space];
        d[..8].copy_from_slice(T::DISCRIMINATOR);
        T::try_deserialize(&mut d.as_slice()).unwrap()
    }

    fn k(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    #[test]
    fn module_listing_wrapper_and_vault_match_the_shared_layout() {
        let mut m: Module = zeroed(Module::INIT_SPACE);
        m.listing = k(1);
        m.wrapper = k(2);
        m.vault = k(3);
        m.params = crate::params::default_params(7);
        m.params.conv_bps = 0x0123;
        m.accepted = 0x0102_0304_0506_0708;
        m.target = u64::MAX;
        let mut d = Vec::new();
        m.try_serialize(&mut d).unwrap();
        assert_eq!(d.len(), rook_layouts::module::LEN);
        assert_eq!(Module::DISCRIMINATOR, &rook_layouts::module::DISCRIMINATOR[..]);
        let v = rook_layouts::module::parse(&d).unwrap();
        assert_eq!(
            (v.listing, v.wrapper, v.vault, v.conv_bps, v.accepted),
            (k(1).to_bytes(), k(2).to_bytes(), k(3).to_bytes(), 0x0123, 0x0102_0304_0506_0708)
        );
        assert!(rook_layouts::module::parse(&d[..d.len() - 1]).is_none());

        let mut w: Wrapper = zeroed(Wrapper::INIT_SPACE);
        w.listing = k(4);
        w.wx_mint = k(5);
        let mut d = Vec::new();
        w.try_serialize(&mut d).unwrap();
        assert_eq!(Wrapper::DISCRIMINATOR, &rook_layouts::wrapper::DISCRIMINATOR[..]);
        let v = rook_layouts::wrapper::parse(&d).unwrap();
        assert_eq!((v.listing, v.wx_mint), (k(4).to_bytes(), k(5).to_bytes()));

        let mut l: Listing = zeroed(Listing::INIT_SPACE);
        l.id = 0x0a0b_0c0d;
        let mut d = Vec::new();
        l.try_serialize(&mut d).unwrap();
        assert_eq!(Listing::DISCRIMINATOR, &rook_layouts::listing::DISCRIMINATOR[..]);
        assert_eq!(rook_layouts::listing::id(&d), Some(0x0a0b_0c0d));

        assert_eq!(ModuleVault::DISCRIMINATOR, &rook_layouts::module_vault::DISCRIMINATOR[..]);
        let mut d = Vec::new();
        ModuleVault {}.try_serialize(&mut d).unwrap();
        assert!(rook_layouts::module_vault::is(&d));

        use crate::seeds as S;
        use rook_layouts::core_seeds as L;
        assert_eq!(
            (S::LISTING, S::MODULE, S::MODULE_VAULT, S::WRAPPER, S::WX_MINT),
            (L::LISTING, L::MODULE, L::MODULE_VAULT, L::WRAPPER, L::WX_MINT)
        );
    }
}
