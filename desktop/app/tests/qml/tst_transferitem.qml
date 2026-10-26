// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// Uses the production row with synthetic aggregate transfer counters.
Item {
    id: root
    width: 520
    height: 160

    TransferItem {
        id: transfer
        width: 500
        transferId: "t1"
        deviceName: "Pixel"
        incoming: false
        recording: false
        title: "holiday.mp4"
        total: 100 * 1024 * 1024
        done: 80 * 1024 * 1024
        progress: 0.8
        rate: 1024 * 1024
        status: "running"
        reason: ""
        savedPath: ""
    }

    TestCase {
        name: "TransferItem"
        when: windowShown

        function init() {
            AppController.reduceMotion = true
            transfer.status = "running"
            transfer.total = 100 * 1024 * 1024
            transfer.done = 80 * 1024 * 1024
            transfer.progress = 0.8
            transfer.rate = 1024 * 1024
        }

        function cleanup() {
            AppController.reduceMotion = true
        }

        function hasText(item, text) {
            if (item.visible === false) return false
            if (item.text === text) return true
            for (let i = 0; i < item.children.length; i++) {
                if (hasText(item.children[i], text)) return true
            }
            return false
        }

        function test_running_transfer_shows_percent_and_rounded_eta() {
            compare(transfer.percent, 80)
            verify(hasText(transfer, "80%"))
            compare(transfer.timeLeft(), "20 s left")
            compare(transfer.detail(), "80 MB of 100 MB · 1.0 MB/s · 20 s left")
            transfer.done = 0
            transfer.progress = 0
            compare(transfer.timeLeft(), "2 min left")
            transfer.done = 99 * 1024 * 1024
            transfer.progress = 0.99
            compare(transfer.timeLeft(), "A few seconds left")
            transfer.total = 4 * 1024 * 1024 * 1024
            transfer.done = 0
            compare(transfer.timeLeft(), "1 h 8 min left")
            for (const reduceMotion of [true, false]) {
                AppController.reduceMotion = reduceMotion
                transfer.done = -1
                transfer.progress = -1
                compare(transfer.percent, 0)
                verify(hasText(transfer, "0%"))
                transfer.done = transfer.total * 2
                transfer.progress = 2
                compare(transfer.percent, 100)
                verify(hasText(transfer, "100%"))
            }
        }

        function test_unavailable_estimates_are_not_shown() {
            for (const status of ["waiting", "paused", "retrying", "failed", "cancelled", "done"]) {
                transfer.status = status
                compare(transfer.timeLeft(), "", status)
                if (["retrying", "failed", "cancelled", "done"].indexOf(status) >= 0)
                    verify(!hasText(transfer, "80%"), status + " has no active percent")
            }
            compare(transfer.detail(), "Sent to Pixel · 100 MB")
            transfer.status = "running"
            for (const rate of [0, -1, NaN, Infinity, -Infinity]) {
                transfer.rate = rate
                compare(transfer.timeLeft(), "", "no estimate from an invalid rate")
            }
            transfer.rate = 1024 * 1024
            for (const total of [0, -1, NaN, Infinity]) {
                transfer.total = total
                compare(transfer.timeLeft(), "", "no estimate from an unknown total")
                verify(!transfer.showPercent, "unknown total has no percent")
            }
            transfer.total = 100 * 1024 * 1024
            for (const done of [NaN, Infinity, -1, transfer.total, transfer.total + 1]) {
                transfer.done = done
                compare(transfer.timeLeft(), "", "no estimate from invalid or complete counters")
            }
        }
    }
}
