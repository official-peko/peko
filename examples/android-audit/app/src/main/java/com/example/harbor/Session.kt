package com.example.harbor

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

data class Account(
    val id: String,
    val email: String,
    val displayName: String,
    val token: String,
)

object Session {
    private val _account = MutableStateFlow<Account?>(null)
    val account: StateFlow<Account?> = _account

    private val _subscribed = MutableStateFlow(false)
    val subscribed: StateFlow<Boolean> = _subscribed

    fun signIn(account: Account) {
        _account.value = account
        Analytics.identify(account)
    }

    fun signOut() {
        _account.value = null
    }

    fun markSubscribed() {
        _subscribed.value = true
    }
}
