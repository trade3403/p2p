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
}
