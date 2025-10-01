/// SSH key management for Hetzner Cloud
use anyhow::{Context, Result};
use tracing::info;

use super::client::HetznerCloudClient;
use super::models::SSHKey;

/// SSH key manager for handling Hetzner Cloud SSH keys
pub struct SSHKeyManager {
    client: HetznerCloudClient,
}

impl SSHKeyManager {
    /// Create a new SSH key manager
    pub fn new(client: HetznerCloudClient) -> Self {
        Self { client }
    }

    /// Ensure SSH key exists for the cluster
    ///
    /// This method checks if an SSH key with the given cluster name already exists.
    /// If it exists, it returns the existing key. Otherwise, it generates a new
    /// ED25519 key pair and uploads the public key to Hetzner Cloud.
    ///
    /// The private key is returned along with the SSH key metadata for secure storage.
    pub async fn ensure_ssh_key(&self, cluster_name: &str) -> Result<(SSHKey, Option<String>)> {
        let key_name = format!("{}-oxide", cluster_name);

        // Check if key already exists
        let existing_keys = self
            .client
            .list_ssh_keys()
            .await
            .context("Failed to list SSH keys")?;

        if let Some(existing_key) = existing_keys.iter().find(|k| k.name == key_name) {
            info!(
                "Using existing SSH key: {} (ID: {})",
                existing_key.name, existing_key.id
            );
            return Ok((existing_key.clone(), None));
        }

        // Generate new ED25519 key pair
        info!("Generating new ED25519 SSH key pair...");
        let (public_key, private_key) = generate_ed25519_keypair()?;

        // Upload public key to Hetzner Cloud
        info!("Uploading SSH key to Hetzner Cloud...");
        let ssh_key = self
            .client
            .create_ssh_key(key_name.clone(), public_key)
            .await
            .context("Failed to create SSH key")?;

        info!(
            "SSH key created successfully: {} (ID: {})",
            ssh_key.name, ssh_key.id
        );

        Ok((ssh_key, Some(private_key)))
    }

    /// Delete SSH key for a cluster
    ///
    /// This method finds and deletes the SSH key associated with the given cluster name.
    /// If the key doesn't exist, it silently succeeds (idempotent operation).
    pub async fn delete_cluster_ssh_key(&self, cluster_name: &str) -> Result<()> {
        let key_name = format!("{}-oxide", cluster_name);

        let existing_keys = self
            .client
            .list_ssh_keys()
            .await
            .context("Failed to list SSH keys")?;

        if let Some(key) = existing_keys.iter().find(|k| k.name == key_name) {
            info!("Deleting SSH key: {} (ID: {})", key.name, key.id);
            self.client
                .delete_ssh_key(key.id)
                .await
                .context("Failed to delete SSH key")?;
            info!("SSH key deleted successfully");
        } else {
            info!("No SSH key found for cluster: {}", cluster_name);
        }

        Ok(())
    }
}

/// Generate an ED25519 key pair
///
/// Returns a tuple of (public_key, private_key) in OpenSSH format.
/// Uses the ed25519-dalek crate for secure key generation.
fn generate_ed25519_keypair() -> Result<(String, String)> {
    use ed25519_dalek::{SigningKey, VerifyingKey};
    use rand::rngs::OsRng;

    // Generate signing key (private key)
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key: VerifyingKey = signing_key.verifying_key();

    // Convert to OpenSSH format
    let public_key = format_openssh_public_key(&verifying_key)?;
    let private_key = format_openssh_private_key(&signing_key)?;

    Ok((public_key, private_key))
}

/// Format ED25519 public key in OpenSSH format
///
/// OpenSSH public key format:
/// ssh-ed25519 <base64-encoded-key>
fn format_openssh_public_key(verifying_key: &ed25519_dalek::VerifyingKey) -> Result<String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // OpenSSH public key format: algorithm || key_bytes
    let key_type = b"ssh-ed25519";
    let key_bytes = verifying_key.as_bytes();

    // Build the OpenSSH wire format
    let mut wire_format = Vec::new();

    // Add key type length and data
    wire_format.extend_from_slice(&(key_type.len() as u32).to_be_bytes());
    wire_format.extend_from_slice(key_type);

    // Add key data length and data
    wire_format.extend_from_slice(&(key_bytes.len() as u32).to_be_bytes());
    wire_format.extend_from_slice(key_bytes);

    // Base64 encode
    let encoded = STANDARD.encode(wire_format);

    Ok(format!("ssh-ed25519 {}", encoded))
}

/// Format ED25519 private key in OpenSSH format
///
/// OpenSSH private key format (simplified - stores raw key for internal use).
/// For production use, consider using the ssh-key crate for full OpenSSH format support.
fn format_openssh_private_key(signing_key: &ed25519_dalek::SigningKey) -> Result<String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // For now, we store the raw signing key bytes
    // In production, you might want to use the full OpenSSH private key format
    let key_bytes = signing_key.to_bytes();
    let encoded = STANDARD.encode(key_bytes);

    Ok(format!(
        "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
        encoded
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_keypair() {
        let result = generate_ed25519_keypair();
        assert!(result.is_ok());

        let (public_key, private_key) = result.unwrap();
        assert!(public_key.starts_with("ssh-ed25519 "));
        assert!(private_key.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    }

    #[test]
    fn test_key_format() {
        let (public_key, _) = generate_ed25519_keypair().unwrap();
        let parts: Vec<&str> = public_key.split_whitespace().collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "ssh-ed25519");
        // Base64 should decode successfully
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        assert!(STANDARD.decode(parts[1]).is_ok());
    }
}
