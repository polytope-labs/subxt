// Copyright 2019-2024 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! An ethereum keypair implementation.

use derive_more::{Display, From};
use keccak_hash::keccak;
use secp256k1::{Message, Secp256k1};

use crate::crypto::{DeriveJunction, SecretUri};
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
    /// Create a keypair from a [`SecretUri`]. See the [`SecretUri`] docs for more.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ SecretUri, eth::Keypair };
    /// use std::str::FromStr;
    ///
    /// let uri = SecretUri::from_str("//Alice").unwrap();
    /// let keypair = Keypair::from_uri(&uri).unwrap();
    ///
    /// keypair.sign(b"Hello world!");
    /// ```
    pub fn from_uri(uri: &SecretUri) -> Result<Self, Error> {
        ecdsa::Keypair::from_uri(uri)
            .map(Self)
            .map_err(Error::Inner)
    }

    /// Create a keypair from a BIP-39 mnemonic phrase and optional password.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ bip39::Mnemonic, eth::Keypair };
    ///
    /// let phrase = "bottom drive obey lake curtain smoke basket hold race lonely fit walk";
    /// let mnemonic = Mnemonic::parse(phrase).unwrap();
    /// let keypair = Keypair::from_phrase(&mnemonic, None).unwrap();
    ///
    /// keypair.sign(b"Hello world!");
    /// ```
    pub fn from_phrase(mnemonic: &bip39::Mnemonic, password: Option<&str>) -> Result<Self, Error> {
        ecdsa::Keypair::from_phrase(mnemonic, password)
            .map(Self)
            .map_err(Error::Inner)
    }

    /// Turn a 32 byte seed into a keypair.
    ///
    /// # Warning
    ///
    /// This will only be secure if the seed is secure!
    pub fn from_seed(seed: Seed) -> Result<Self, Error> {
        ecdsa::Keypair::from_seed(seed)
            .map(Self)
            .map_err(Error::Inner)
    }

    /// Derive a child key from this one given a series of junctions.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ bip39::Mnemonic, eth::Keypair, DeriveJunction };
    ///
    /// let phrase = "bottom drive obey lake curtain smoke basket hold race lonely fit walk";
    /// let mnemonic = Mnemonic::parse(phrase).unwrap();
    /// let keypair = Keypair::from_phrase(&mnemonic, None).unwrap();
    ///
    /// // Equivalent to the URI path '//Alice//stash':
    /// let new_keypair = keypair.derive([
    ///     DeriveJunction::hard("Alice"),
    ///     DeriveJunction::hard("stash")
    /// ]);
    /// ```
    pub fn derive<Js: IntoIterator<Item = DeriveJunction>>(
        &self,
        junctions: Js,
    ) -> Result<Self, Error> {
        self.0.derive(junctions).map(Self).map_err(Error::Inner)
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
        Signature(crate::ecdsa::sign(&self.0 .0.secret_key(), &wrapped))
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

/// Verify that some signature for a message was created by the owner of the [`ecdsa::PublicKey`].
///
/// ```rust
/// use subxt_signer::eth;
///
/// let keypair = eth::dev::alice();
/// let message = b"Hello!";
///
/// let signature = keypair.sign(message);
/// let public_key = keypair.public_key();
/// assert!(eth::verify(&signature, message, &public_key));
/// ```
pub fn verify<M: AsRef<[u8]>>(sig: &Signature, message: M, pub_key: &ecdsa::PublicKey) -> bool {
    let Ok(signature) = secp256k1::ecdsa::Signature::from_compact(&sig.0[..64]) else {
        return false;
    };
    let message_hash = keccak(message.as_ref());
    let wrapped =
        Message::from_digest_slice(message_hash.as_bytes()).expect("Message is 32 bytes; qed");
    let pub_key = secp256k1::PublicKey::from_slice(&pub_key.0).expect("valid public key");

    Secp256k1::verification_only()
        .verify_ecdsa(&wrapped, &signature, &pub_key)
        .is_ok()
}

/// An error handed back if creating the keypair fails.
#[derive(Debug, PartialEq, Display, From)]
pub enum Error {
    /// Invalid private key.
    #[display(fmt = "Invalid private key")]
    #[from(ignore)]
    InvalidPrivateKey,
    /// Invalid hex.
    #[display(fmt = "Cannot parse hex string: {_0}")]
    Hex(hex::FromHexError),
    /// Inner,
    #[display(fmt = "{_0}")]
    Inner(ecdsa::Error),
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Dev accounts, helpful for testing but not to be used in production,
/// since the secret keys are known.
pub mod dev {
    use super::*;
    use core::str::FromStr;

    once_static_cloned! {
        /// Equivalent to `{DEV_PHRASE}//Alice`.
        pub fn alice() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Alice").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Bob`.
        pub fn bob() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Bob").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Charlie`.
        pub fn charlie() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Charlie").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Dave`.
        pub fn dave() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Dave").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Eve`.
        pub fn eve() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Eve").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Ferdie`.
        pub fn ferdie() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Ferdie").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//One`.
        pub fn one() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//One").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Two`.
        pub fn two() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Two").unwrap()).unwrap()
        }
    }
}

#[cfg(feature = "subxt")]
mod subxt_compat {
    use super::*;

    impl<T: subxt::Config> subxt::tx::Signer<T> for Keypair
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
    use super::*;
    use proptest::prelude::*;

    enum StubEthRuntimeConfig {}

    impl subxt::Config for StubEthRuntimeConfig {
        type Hash = subxt::utils::H256;
        type AccountId = super::AccountId20;
        type Address = super::AccountId20;
        type Signature = super::Signature;
        type Hasher = subxt::config::substrate::BlakeTwo256;
        type Header =
            subxt::config::substrate::SubstrateHeader<u32, subxt::config::substrate::BlakeTwo256>;
        type ExtrinsicParams = subxt::config::SubstrateExtrinsicParams<Self>;
        type AssetId = u32;
    }

    type Signer = dyn subxt::tx::Signer<StubEthRuntimeConfig>;

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

            assert_eq!(Signer::account_id(&keypair), keypair.account_id());
            assert_eq!(Signer::sign(&keypair, msg_as_bytes), keypair.sign(msg_as_bytes));
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
            assert_eq!(Signer::account_id(&keypair), Signer::address(&keypair));
        }

        #[test]
        fn check_signing_and_verifying_matches(keypair in keypair(), msg in ".*") {
            let sig = Signer::sign(&keypair, msg.as_bytes());

            assert!(verify(
                &sig,
                msg,
                &keypair.public_key())
            );
        }
    }
}
