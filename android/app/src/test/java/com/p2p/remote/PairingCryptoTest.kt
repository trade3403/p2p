package com.p2p.remote

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.SecureRandom
import java.util.Base64

class PairingCryptoTest {
    @Test fun hostAndViewerDeriveIdenticalProofAndSessionSecret() {
        val host = PairingCrypto.generatePairingWindow(1_790_000_000L)
        val viewer = PairingCrypto.generateViewerEphemeral()
        val nonce = ByteArray(16).also(SecureRandom()::nextBytes)
        val hostPublic = host.publicKey
        val viewerProof = PairingCrypto.proofForViewer(
            viewer, "123456789", hostPublic, nonce, host.code
        )
        val hostProof = PairingCrypto.proofForHost(
            host, "123456789", viewer.publicKey, nonce
        )
        assertArrayEquals(hostProof, viewerProof)
        val viewerSession = PairingCrypto.sessionSecretForViewer(
            viewer, "123456789", hostPublic, nonce
        )
        val hostSession = PairingCrypto.sessionSecretForHost(
            host, "123456789", viewer.publicKey, nonce
        )
        assertArrayEquals(hostSession, viewerSession)
        assertEquals(32, viewerSession.size)
    }

    @Test fun incorrectCodeProducesDifferentProof() {
        val host = PairingCrypto.generatePairingWindow(1_790_000_000L)
        val viewer = PairingCrypto.generateViewerEphemeral()
        val nonce = ByteArray(16).also(SecureRandom()::nextBytes)
        val correct = PairingCrypto.proofForViewer(
            viewer, "123456789", host.publicKey, nonce, host.code
        )
        val wrongCode = ((host.code.toInt() + 1) % 1_000_000).toString().padStart(6, '0')
        val wrong = PairingCrypto.proofForViewer(
            viewer, "123456789", host.publicKey, nonce, wrongCode
        )
        assertNotEquals(correct.toList(), wrong.toList())
    }

    @Test fun identitySignatureAuthenticatesPairingWindow() {
        val identity = PairingCrypto.generateIdentity()
        val window = PairingCrypto.generatePairingWindow(1_790_000_000L)
        val publicBase64 = PairingCrypto.publicKeyBase64(window.publicKey)
        val signature = PairingCrypto.signPairingWindow(
            identity.privateSeed, "123456789", window.expiresAt, publicBase64
        )
        assertTrue(PairingCrypto.verifyPairingWindow(
            identity.publicKey, "123456789", window.expiresAt, publicBase64, signature
        ))
        assertFalse(PairingCrypto.verifyPairingWindow(
            identity.publicKey, "123456788", window.expiresAt, publicBase64, signature
        ))
    }

    @Test fun deviceIdentityIsNineDigitCompatibleAndHasRandomId() {
        val first = PairingCrypto.generateIdentity()
        val second = PairingCrypto.generateIdentity()
        assertEquals(32, first.deviceId.length)
        assertNotEquals(first.deviceId, second.deviceId)
        assertEquals(32, first.publicKey.size)
        assertEquals(32, first.privateSeed.size)
    }
}
