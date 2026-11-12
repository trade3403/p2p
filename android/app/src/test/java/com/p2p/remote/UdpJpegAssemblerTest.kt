package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

class UdpJpegAssemblerTest {
    private val key = ByteArray(32) { (it+1).toByte() }

    private fun packet(id: Int, part: Int, data: ByteArray): ByteArray {
        val count = (data.size + 899) / 900
        val body = ByteBuffer.allocate(12 + minOf(900, data.size - part * 900))
            .putInt(id)
            .putShort(count.toShort())
            .putShort(part.toShort())
            .putInt(data.size)
            .put(data, part * 900, minOf(900, data.size - part * 900))
            .array()
        val iv = ByteArray(12) { (it + part + id).toByte() }
        val encrypt = Cipher.getInstance("AES/GCM/NoPadding")
        encrypt.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key,"AES"),GCMParameterSpec(128,iv))
        encrypt.updateAAD(byteArrayOf(14))
        return byteArrayOf(14) + iv + encrypt.doFinal(body)
    }

    @Test fun reordersFragmentsAndRejectsReplay() {
        val jpeg = ByteArray(2350) { (it%251).toByte() }
        jpeg[0] = 0xff.toByte(); jpeg[1] = 0xd8.toByte()
        jpeg[jpeg.size-2] = 0xff.toByte(); jpeg[jpeg.size-1] = 0xd9.toByte()
        val parts = (0..2).map { packet(1,it,jpeg) }
        val assembler = UdpJpegAssembler()
        assertNull(assembler.accept(parts[2],key,1000))
        assertNull(assembler.accept(parts[0],key,1010))
        assertArrayEquals(jpeg,assembler.accept(parts[1],key,1020))
        assertNull("already completed image must not replay",assembler.accept(parts[0],key,1030))
    }

    @Test fun rejectsWrongKeyTamperAndMissingParts() {
        val data = ByteArray(1800){5}
        data[0] = 0xff.toByte();data[1]=0xd8.toByte()
        data[1798]=0xff.toByte();data[1799]=0xd9.toByte()
        val a=UdpJpegAssembler()
        val p0=packet(2,0,data)
        assertNull(a.accept(p0,ByteArray(32),0))
        val changed=p0.copyOf();changed[changed.lastIndex]=(changed.last().toInt() xor 1).toByte()
        assertNull(a.accept(changed,key,0))
        assertNull(a.accept(p0,key,100))
        assertNull("late chunk must not produce frame",a.accept(packet(2,1,data),key,2000))
    }
}
