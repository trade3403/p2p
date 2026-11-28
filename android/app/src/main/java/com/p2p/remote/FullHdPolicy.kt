package com.p2p.remote

/** Main-thread, socket-local policy. Capability declarations are not measured FPS. */
internal class FullHdPolicy {
    enum class State { LEGACY, REQUESTED, ACTIVE, DOWNGRADED }
    var state=State.LEGACY
        private set
    private var requestedAt=0L
    fun reset() { state=State.LEGACY;requestedAt=0L }
    fun canAttempt()=state==State.LEGACY
    fun request(now:Long):Boolean {
        if(!canAttempt()) return false
        requestedAt=now
        state=State.REQUESTED
        return true
    }
    fun rendered(highResolution:Boolean) {
        if(highResolution && state==State.REQUESTED) state=State.ACTIVE
    }
    fun expired(now:Long)=state==State.REQUESTED && now-requestedAt>=12_000L
    fun requiresHardware()=state==State.REQUESTED || state==State.ACTIVE
    fun downgrade():Boolean {
        if(!requiresHardware()) return false
        state=State.DOWNGRADED
        return true
    }
    fun rejectRetiredFormat(w:Int,h:Int)=state==State.DOWNGRADED && (w>1280 || h>720)
}
