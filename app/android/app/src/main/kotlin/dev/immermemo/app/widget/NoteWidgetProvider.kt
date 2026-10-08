package dev.immermemo.app.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.RemoteViews
import dev.immermemo.app.R

/** The extra key [NoteWidgetFactory] puts a tapped note's vault-relative path under. */
const val EXTRA_NOTE_PATH = "note_path"

/**
 * Home-screen widget showing the most recently modified notes (data comes
 * from a JSON snapshot the Rust side writes -- see
 * `app/src/widget.rs`'s `export_recent_notes`). A plain
 * [AppWidgetProvider]; the actual list is backed by [NoteWidgetService]'s
 * [android.widget.RemoteViewsService.RemoteViewsFactory], since a static
 * layout can't show a variable-length list.
 */
class NoteWidgetProvider : AppWidgetProvider() {
    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray,
    ) {
        for (appWidgetId in appWidgetIds) {
            val views = RemoteViews(context.packageName, R.layout.widget_recent_notes)

            // `data` must be unique per widget instance -- otherwise Android
            // treats every instance's adapter-connecting Intent as the same
            // one and they'd all end up sharing one `RemoteViewsFactory`.
            val serviceIntent =
                Intent(context, NoteWidgetService::class.java).apply {
                    data = Uri.parse("widget://dev.immermemo.app/$appWidgetId")
                }
            views.setRemoteAdapter(R.id.widget_list, serviceIntent)
            views.setEmptyView(R.id.widget_list, R.id.widget_empty)

            // The template PendingIntent must be mutable: the system fills
            // in each row's own extras (set via `setOnClickFillInIntent` in
            // NoteWidgetFactory) into a copy of this one at click time.
            // Deliberately NOT `FLAG_ACTIVITY_CLEAR_TASK`: that would force
            // `android.app.NativeActivity` to be destroyed and recreated
            // (re-running `android_main` in the same process, re-hitting
            // every `OnceLock` in `app/src/widget.rs`/`haptic.rs`,
            // which only accept the first caller) even when the app is
            // already running. With plain `singleTask`, a cold start still
            // opens straight to the tapped note (see
            // `widget::read_launch_note_path`); a warm start just brings
            // the running app to the foreground as-is -- the stock
            // `NativeActivity` has no `onNewIntent` override to relay a
            // second tap into, and nothing here attempts to fake one.
            val launchIntent =
                Intent(Intent.ACTION_VIEW).apply {
                    setClassName(context, "android.app.NativeActivity")
                    flags = Intent.FLAG_ACTIVITY_NEW_TASK
                }
            val launchPendingIntent =
                PendingIntent.getActivity(
                    context,
                    0,
                    launchIntent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                )
            views.setPendingIntentTemplate(R.id.widget_list, launchPendingIntent)

            appWidgetManager.updateAppWidget(appWidgetId, views)
        }
    }

    companion object {
        /**
         * Called from Rust (`app/src/widget.rs`'s `export_recent_notes`,
         * over JNI) right after it rewrites the snapshot file, so the widget
         * reflects a change immediately instead of waiting for
         * `appwidget_info.xml`'s 30-minute `updatePeriodMillis` floor.
         */
        @JvmStatic
        fun requestUpdate(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, NoteWidgetProvider::class.java))
            if (ids.isNotEmpty()) {
                manager.notifyAppWidgetViewDataChanged(ids, R.id.widget_list)
            }
        }
    }
}
