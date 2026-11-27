package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class AvcDecodeQueueTest {
    @Test fun overflowIsOrderedAfterAlreadyAdmittedUnits() {
        val queue = AvcDecodeQueue()
        val a = queue.admit()!!
        val b = queue.admit()!!
        assertEquals(a, b)
        assertNull(queue.admit())
        assertNull(queue.admit())
        queue.complete(a)
        val afterLoss = queue.admit()!!
        assertEquals(a.generation, afterLoss.generation)
        assertEquals(a.lossSerial + 2, afterLoss.lossSerial)
        // Completing older tasks must not clear the loss serial.
        queue.complete(b)
        val following = queue.admit()!!
        assertEquals(afterLoss, following)
        queue.complete(afterLoss)
        queue.complete(following)
        assertEquals(2L, queue.droppedFrames)
    }

    @Test fun oldCompletionsDoNotConsumeNewGenerationCapacity() {
        val queue = AvcDecodeQueue()
        val oldA = queue.admit()!!
        val oldB = queue.admit()!!
        assertTrue(queue.invalidate())
        val freshA = queue.admit()!!
        val freshB = queue.admit()!!
        assertNotEquals(oldA.generation, freshA.generation)
        assertFalse(queue.isCurrent(oldA.generation))
        queue.complete(oldA)
        queue.complete(oldB)
        assertNull(queue.admit())
        queue.complete(freshA)
        assertNotNull(queue.admit())
        queue.complete(freshB)
    }

    @Test fun invalidationAndCloseRejectStaleRenderAndDropUpdates() {
        val queue = AvcDecodeQueue()
        val old = queue.admit()!!
        var renders = 0
        assertTrue(queue.ifCurrent(old.generation) { renders++ })
        assertTrue(queue.invalidate())
        assertFalse(queue.ifCurrent(old.generation) { renders++ })
        queue.recordWorkerDrop(old.generation)
        assertEquals(0L, queue.droppedFrames)
        val fresh = queue.admit()!!
        queue.recordWorkerDrop(fresh.generation)
        assertEquals(1L, queue.droppedFrames)
        assertTrue(queue.ifCurrent(fresh.generation) { renders++ })
        assertTrue(queue.close())
        assertFalse(queue.close())
        assertFalse(queue.invalidate())
        assertFalse(queue.ifCurrent(fresh.generation) { renders++ })
        assertNull(queue.admit())
        queue.complete(fresh)
        assertEquals(2, renders)
        assertEquals(1L, queue.droppedFrames)
    }

    @Test fun resolutionChangeRetiresOldQueueAndRenderEvenAtCapacity() {
        val queue = AvcDecodeQueue()
        val oldA = queue.admit(960,720)!!
        val oldB = queue.admit(960,720)!!
        val newA = queue.admit(1280,720)!!
        assertNotEquals(oldA.generation, newA.generation)
        assertFalse(queue.ifCurrent(oldA.generation) { fail("old resolution rendered") })
        assertEquals(0L, queue.droppedFrames)
        queue.complete(oldA)
        queue.complete(oldB)
        val newB = queue.admit(1280,720)!!
        assertEquals(newA, newB)
        assertNull(queue.admit(1280,720))
        // Switching back is another generation, not a revival of old tasks.
        val back = queue.admit(960,720)!!
        assertNotEquals(oldA.generation, back.generation)
        assertNotEquals(newA.generation, back.generation)
        assertEquals(0L, back.lossSerial)
        queue.complete(newA)
        queue.complete(newB)
        assertTrue(queue.ifCurrent(back.generation) {})
        queue.complete(back)
        assertEquals(1L, queue.droppedFrames)
    }

    @Test fun outputIdentitySurvivesSameGenerationRecoveryButNotReset() {
        val queue = AvcDecodeQueue()
        val a = queue.admit(960,720)!!
        val software = AvcDecodeQueue.DecoderIdentity("software", false)
        val hardware = AvcDecodeQueue.DecoderIdentity("hardware", true)
        assertNull(queue.outputState().lastOutput)
        assertTrue(queue.outputState().recovering)
        assertTrue(queue.submitOutput(a.generation, software) {})
        assertEquals(AvcDecodeQueue.OutputState(software, false),queue.outputState())
        queue.recordWorkerDrop(a.generation)
        assertEquals(AvcDecodeQueue.OutputState(software, true),queue.outputState())
        queue.beginRecovery(a.generation)
        assertEquals(AvcDecodeQueue.OutputState(software, true),queue.outputState())
        // No candidate identity until it has actually submitted an output.
        assertTrue(queue.submitOutput(a.generation, hardware) {})
        assertEquals(AvcDecodeQueue.OutputState(hardware, false),queue.outputState())
        val b = queue.admit(1280,720)!!
        assertNull(queue.outputState().lastOutput)
        assertTrue(queue.outputState().recovering)
        assertFalse(queue.submitOutput(a.generation, software) { fail("stale output") })
        queue.beginRecovery(a.generation)
        assertTrue(queue.submitOutput(b.generation, software) {})
        assertEquals(AvcDecodeQueue.OutputState(software, false),queue.outputState())
        queue.invalidate()
        assertNull(queue.outputState().lastOutput)
        assertTrue(queue.outputState().recovering)
        assertFalse(queue.submitOutput(b.generation, hardware) { fail("retired output") })
        assertTrue(queue.close())
        assertNull(queue.outputState().lastOutput)
    }

    @Test fun failedSurfaceSubmissionDoesNotPublishCandidateIdentity() {
        val queue = AvcDecodeQueue()
        val a = queue.admit()!!
        val identity = AvcDecodeQueue.DecoderIdentity("candidate", null)
        try {
            queue.submitOutput(a.generation, identity) { throw IllegalStateException("test") }
            fail("expected failed submission")
        } catch (_: IllegalStateException) {
            assertNull(queue.outputState().lastOutput)
            assertTrue(queue.outputState().recovering)
        }
    }
}
