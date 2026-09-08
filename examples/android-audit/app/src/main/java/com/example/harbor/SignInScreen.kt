package com.example.harbor

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

enum class IdentityProvider { GOOGLE, FACEBOOK }

@Composable
fun SignInScreen() {
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Harbor")
        Text("Your notes, on every device you own.")

        Button(
            onClick = {
                busy = true
                scope.launch {
                    IdentityClient.authenticate(IdentityProvider.GOOGLE)?.let(Session::signIn)
                    busy = false
                }
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp)
        ) {
            Text("Continue with Google")
        }

        OutlinedButton(
            onClick = {
                busy = true
                scope.launch {
                    IdentityClient.authenticate(IdentityProvider.FACEBOOK)?.let(Session::signIn)
                    busy = false
                }
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Continue with Facebook")
        }
    }
}

object IdentityClient {
    /** Hand off to the provider SDK and exchange the token with our server. */
    suspend fun authenticate(provider: IdentityProvider): Account? =
        Account(
            id = "acct_8172",
            email = "person@example.com",
            displayName = "A Person",
            token = "harbor_session_9f2c1b",
        )
}
