package com.example.harbor

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.dp

private const val FREE_NOTE_LIMIT = 10

data class Note(val id: Long, val title: String, val body: String)

@Composable
fun NoteListScreen(onSettings: () -> Unit) {
    val notes = remember { mutableStateListOf<Note>() }
    val subscribed by Session.subscribed.collectAsState()

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Notes")
        TextButton(onClick = onSettings) { Text("Settings") }

        Button(onClick = {
            if (subscribed || notes.size < FREE_NOTE_LIMIT) {
                notes.add(Note(System.currentTimeMillis(), "New note", ""))
            } else {
                Billing.startPurchase()
            }
        }) {
            Text("New note")
        }

        LazyColumn {
            items(notes) { note -> Text(note.title) }
        }
    }
}
