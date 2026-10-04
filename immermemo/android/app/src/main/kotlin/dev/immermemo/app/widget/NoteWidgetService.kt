package dev.immermemo.app.widget

import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import android.widget.RemoteViewsService
import dev.immermemo.app.R
import java.io.File
import org.json.JSONArray

class NoteWidgetService : RemoteViewsService() {
    override fun onGetViewFactory(intent: Intent): RemoteViewsFactory =
        NoteWidgetFactory(applicationContext)
}

/**
 * Reads `widget_recent_notes.json` from the app's own `filesDir` (the same
 * directory `android_activity::AndroidApp::internal_data_path()` hands
 * Rust) and renders one row per note. No caching beyond [notes] itself:
 * [onDataSetChanged] is called by the widget host before every refresh, so
 * re-reading the (small) file each time keeps this honest rather than
 * chasing a staleness bug.
 */
private class NoteWidgetFactory(private val context: Context) : RemoteViewsService.RemoteViewsFactory {
    private data class Note(val path: String, val title: String)

    private var notes: List<Note> = emptyList()

    override fun onCreate() {}

    override fun onDataSetChanged() {
        notes = loadNotes()
    }

    override fun onDestroy() {
        notes = emptyList()
    }

    override fun getCount(): Int = notes.size

    override fun getViewAt(position: Int): RemoteViews {
        val note = notes[position]
        val views = RemoteViews(context.packageName, R.layout.widget_list_item)
        views.setTextViewText(R.id.widget_item_title, note.title)
        views.setOnClickFillInIntent(
            R.id.widget_item_title,
            Intent().putExtra(EXTRA_NOTE_PATH, note.path),
        )
        return views
    }

    override fun getLoadingView(): RemoteViews? = null

    override fun getViewTypeCount(): Int = 1

    override fun getItemId(position: Int): Long = notes[position].path.hashCode().toLong()

    override fun hasStableIds(): Boolean = true

    private fun loadNotes(): List<Note> {
        val file = File(context.filesDir, "widget_recent_notes.json")
        if (!file.exists()) return emptyList()
        return try {
            val array = JSONArray(file.readText())
            (0 until array.length()).map { i ->
                val obj = array.getJSONObject(i)
                Note(path = obj.getString("path"), title = obj.getString("title"))
            }
        } catch (e: Exception) {
            emptyList()
        }
    }
}
