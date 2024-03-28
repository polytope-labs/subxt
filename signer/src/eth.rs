// Copyright 2019-2024 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! An ethereum keypair implementation.

use core::fmt::{Display, Formatter};

use derive_more::{Display, From};
use keccak_hash::keccak;
use secp256k1::Message;

use crate::ecdsa;

const SEED_LENGTH: usize = 32;

/// Seed bytes used to generate a key pair.
pub type Seed = [u8; SEED_LENGTH];

/// An ethereum keypair implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keypair(ecdsa::Keypair);

impl From<ecdsa::Keypair> for Keypair {
    fn from(kp: ecdsa::Keypair) -> Self {
        Self(kp)
    }
}

impl Keypair {
    /// Create a keypair from a BIP-39 mnemonic phrase, optional password, and derivation index.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ bip39::Mnemonic, eth::Keypair };
    ///
    /// let phrase = "bottom drive obey lake curtain smoke basket hold race lonely fit walk";
    /// let mnemonic = Mnemonic::parse(phrase).unwrap();
    /// let keypair = Keypair::from_phrase(&mnemonic, None, 0).unwrap();
    ///
    /// keypair.sign(b"Hello world!");
    /// ```
    pub fn from_phrase(
        mnemonic: &bip39::Mnemonic,
        password: Option<&str>,
        index: u32,
    ) -> Result<Self, Error> {
        let derivation_path: bip32::DerivationPath = format!("m/44'/60'/0'/0/{}", index)
            .parse()
            .map_err(Error::InvalidDerivationIndex)?;
        let private = bip32::XPrv::derive_from_path(
            mnemonic.to_seed(password.unwrap_or("")),
            &derivation_path,
        )
        .unwrap();

        Keypair::from_seed(private.to_bytes())
    }

    /// Turn a 32 byte seed into a keypair.
    ///
    /// # Warning
    ///
    /// This will only be secure if the seed is secure!
    pub fn from_seed(seed: Seed) -> Result<Self, Error> {
        ecdsa::Keypair::from_seed(seed)
            .map(Self)
            .map_err(|_| Error::InvalidSeed)
    }

    /// Obtain the [`ecdsa::PublicKey`] of this keypair.
    pub fn public_key(&self) -> ecdsa::PublicKey {
        self.0.public_key()
    }

    /// Obtains the public address of the account by taking the last 20 bytes
    /// of the Keccak-256 hash of the public key.
    pub fn account_id(&self) -> AccountId20 {
        let uncompressed = self.0 .0.public_key().serialize_uncompressed();
        let hash = keccak(&uncompressed[1..]).0;
        let hash20 = hash[12..].try_into().expect("should be 20 bytes");
        AccountId20(hash20)
    }

    /// Signs an arbitrary message payload.
    pub fn sign(&self, signer_payload: &[u8]) -> Signature {
        let message_hash = keccak(signer_payload);
        let wrapped =
            Message::from_digest_slice(message_hash.as_bytes()).expect("Message is 32 bytes; qed");
        Signature(ecdsa::internal::sign(&self.0 .0.secret_key(), &wrapped))
    }
}

/// A signature generated by [`Keypair::sign()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, codec::Encode)]
pub struct Signature(pub [u8; 65]);

impl AsRef<[u8; 65]> for Signature {
    fn as_ref(&self) -> &[u8; 65] {
        &self.0
    }
}

/// A 20-byte cryptographic identifier.
#[derive(Debug, Copy, Clone, PartialEq, Eq, codec::Encode)]
pub struct AccountId20(pub [u8; 20]);

impl AsRef<[u8]> for AccountId20 {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Display for AccountId20 {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", eth_checksum::checksum(&hex::encode(&self)))
    }
}

pub fn verify<M: AsRef<[u8]>>(sig: &Signature, message: M, pubkey: &ecdsa::PublicKey) -> bool {
    let message_hash = keccak(message.as_ref());
    let wrapped =
        Message::from_digest_slice(message_hash.as_bytes()).expect("Message is 32 bytes; qed");

    ecdsa::internal::verify(&sig.0, &wrapped, pubkey)
}

/// An error handed back if creating a keypair fails.
#[derive(Debug, PartialEq, Display, From)]
pub enum Error {
    /// Invalid seed.
    #[display(fmt = "Invalid seed (was it the wrong length?)")]
    #[from(ignore)]
    InvalidSeed,
    /// Invalid derivation index.
    #[display(fmt = "Invalid derivation index: {_0}")]
    InvalidDerivationIndex(bip32::Error),
    /// Invalid phrase.
    #[display(fmt = "Cannot parse phrase: {_0}")]
    InvalidPhrase(bip39::Error),
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Dev accounts, helpful for testing but not to be used in production,
/// since the secret keys are known.
pub mod dev {
    use core::str::FromStr;

    use crate::DEV_PHRASE;

    use super::*;

    once_static_cloned! {
        pub fn alith() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 0).unwrap()
        }
        pub fn baltathar() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 1).unwrap()
        }
        pub fn charleth() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 2).unwrap()
        }
        pub fn dorothy() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 3).unwrap()
        }
        pub fn ethan() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 4).unwrap()
        }
        pub fn faith() -> Keypair {
            Keypair::from_phrase(
                &bip39::Mnemonic::from_str(DEV_PHRASE).unwrap(), None, 5).unwrap()
        }
    }
}

#[cfg(feature = "subxt")]
mod subxt_compat {
    use super::*;

    use subxt_core::config::Config;
    use subxt_core::tx::Signer as SignerT;
    impl<T: Config> SignerT<T> for Keypair
    where
        T::AccountId: From<AccountId20>,
        T::Address: From<AccountId20>,
        T::Signature: From<Signature>,
    {
        fn account_id(&self) -> T::AccountId {
            self.account_id().into()
        }

        fn address(&self) -> T::Address {
            self.account_id().into()
        }

        fn sign(&self, signer_payload: &[u8]) -> T::Signature {
            self.sign(signer_payload).into()
        }
    }
}

#[cfg(test)]
mod test {
    use core::str::FromStr;

    use proptest::prelude::*;
    use secp256k1::Secp256k1;

    use crate::DEV_PHRASE;
    use subxt_core::{config::*, tx::Signer as SignerT, utils::H256};

    use super::*;

    enum StubEthRuntimeConfig {}

    impl Config for StubEthRuntimeConfig {
        type Hash = H256;
        type AccountId = AccountId20;
        type Address = AccountId20;
        type Signature = Signature;
        type Hasher = substrate::BlakeTwo256;
        type Header = substrate::SubstrateHeader<u32, substrate::BlakeTwo256>;
        type ExtrinsicParams = SubstrateExtrinsicParams<Self>;
        type AssetId = u32;
    }

    type SubxtSigner = dyn SignerT<StubEthRuntimeConfig>;

    prop_compose! {
        fn keypair()(seed in any::<[u8; 32]>()) -> Keypair {
            let secret = secp256k1::SecretKey::from_slice(&seed).expect("valid secret key");
            let inner = secp256k1::Keypair::from_secret_key(
                &Secp256k1::new(),
                &secret,
            );

            Keypair(ecdsa::Keypair(inner))
        }
    }

    proptest! {
        #[test]
        fn check_subxt_signer_implementation_matches(keypair in keypair(), msg in ".*") {
            let msg_as_bytes = msg.as_bytes();

            assert_eq!(SubxtSigner::account_id(&keypair), keypair.account_id());
            assert_eq!(SubxtSigner::sign(&keypair, msg_as_bytes), keypair.sign(msg_as_bytes));
        }

        #[test]
        fn check_account_id(keypair in keypair()) {
            let account_id = {
                let uncompressed = keypair.0.0.public_key().serialize_uncompressed();
                let hash = keccak(&uncompressed[1..]).0;
                let hash20 = hash[12..].try_into().expect("should be 20 bytes");
                AccountId20(hash20)
            };

            assert_eq!(keypair.account_id(), account_id);

        }

        #[test]
        fn check_account_id_eq_address(keypair in keypair()) {
            assert_eq!(SubxtSigner::account_id(&keypair), SubxtSigner::address(&keypair));
        }

        #[test]
        fn check_signing_and_verifying_matches(keypair in keypair(), msg in ".*") {
            let sig = SubxtSigner::sign(&keypair, msg.as_bytes());

            assert!(verify(
                &sig,
                msg,
                &keypair.public_key())
            );
        }
    }

    #[test]
    fn check_dev_accounts_match() {
        assert_eq!(
            dev::alith().account_id().to_string(),
            eth_checksum::checksum("0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac")
        );
        assert_eq!(
            dev::baltathar().account_id().to_string(),
            eth_checksum::checksum("0x3Cd0A705a2DC65e5b1E1205896BaA2be8A07c6e0")
        );
        assert_eq!(
            dev::charleth().account_id().to_string(),
            eth_checksum::checksum("0x798d4Ba9baf0064Ec19eB4F0a1a45785ae9D6DFc")
        );
        assert_eq!(
            dev::dorothy().account_id().to_string(),
            eth_checksum::checksum("0x773539d4Ac0e786233D90A233654ccEE26a613D9")
        );
        assert_eq!(
            dev::ethan().account_id().to_string(),
            eth_checksum::checksum("0xFf64d3F6efE2317EE2807d223a0Bdc4c0c49dfDB")
        );
        assert_eq!(
            dev::faith().account_id().to_string(),
            eth_checksum::checksum("0xC0F0f4ab324C46e55D02D0033343B4Be8A55532d")
        );
    }
}
