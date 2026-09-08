package com.example.harbor

import com.android.billingclient.api.BillingClient
import com.android.billingclient.api.Purchase

/** Harbor Pro is one monthly subscription, sold through Play Billing. */
object Billing {
    const val PRODUCT_ID = "harbor_pro_monthly"

    private var client: BillingClient? = null

    fun startPurchase() {
        // The real flow launches the Play billing sheet for PRODUCT_ID and
        // waits for onPurchasesUpdated.
        Analytics.track("paywall_shown")
    }

    fun onPurchase(purchase: Purchase) {
        if (purchase.purchaseState == Purchase.PurchaseState.PURCHASED) {
            Session.markSubscribed()
            Analytics.track("subscribed")
        }
    }
}
