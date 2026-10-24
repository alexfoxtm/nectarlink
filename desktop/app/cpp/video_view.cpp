// SPDX-License-Identifier: GPL-3.0-or-later
#include "video_view.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QEvent>
#include <QtCore/QHash>
#include <QtCore/QThread>
#include <QtCore/QUrl>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QInputMethod>
#include <QtGui/QInputMethodEvent>
#include <QtGui/QPixmap>
#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/QSGTexture>

#ifdef _WIN32
#include <windows.h>
#include <propkey.h>
#include <propsys.h>
#include <shobjidl.h>
#endif

#include <algorithm>
#include <array>
#include <mutex>
#include <unordered_set>
#include <vector>

namespace {

struct Latest
{
    QImage image;
    quint64 serial = 0;
};

// Pictures by stream, and the views to tell. Views register and leave
// under the lock, so a picture never wakes a view that's being destroyed.
std::mutex lock;
QHash<QString, Latest> latest;
std::vector<VideoView *> views;
// At most one queued GUI wake per live view; images still use the latest slot.
std::unordered_set<VideoView *> queuedFrameUpdates;
quint64 nextSerial = 1;

void wakeViews(const QString &stream)
{
    for (VideoView *view : views) {
        if (view->stream() != stream || !queuedFrameUpdates.insert(view).second)
            continue;
        if (!QMetaObject::invokeMethod(view, "frameArrived", Qt::QueuedConnection))
            queuedFrameUpdates.erase(view);
    }
}

// Owns its texture (replaced with each picture).
class VideoNode : public QSGSimpleTextureNode
{
public:
    VideoNode() { setOwnsTexture(false); }
    ~VideoNode() override { delete texture(); }

    void replaceTexture(QSGTexture *next)
    {
        QSGTexture *previous = texture();
        setTexture(next);
        delete previous;
    }
};

} // namespace

VideoView::VideoView(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    std::lock_guard guard(lock);
    views.push_back(this);
}

VideoView::~VideoView()
{
    std::lock_guard guard(lock);
    queuedFrameUpdates.erase(this);
    views.erase(std::remove(views.begin(), views.end(), this), views.end());
}

void VideoView::setStream(const QString &stream)
{
    if (stream == m_stream)
        return;
    {
        std::lock_guard guard(lock);
        m_stream = stream;
    }
    m_shown = 0;
    emit streamChanged();
    // Keep the synchronous stream refresh without consuming an older queued wake.
    refreshFrame(false);
}

void VideoView::setWindowIcon(const QString &icon)
{
    if (icon == m_windowIcon)
        return;
    m_windowIcon = icon;
    emit windowIconChanged();
    applyWindowChrome();
}

void VideoView::setWindowAppId(const QString &appId)
{
    if (appId == m_windowAppId)
        return;
    m_windowAppId = appId;
    emit windowAppIdChanged();
    applyWindowChrome();
}

void VideoView::setAcceptsText(bool on)
{
    if (on == m_acceptsText)
        return;
    m_acceptsText = on;
    setFlag(ItemAcceptsInputMethod, on);
    if (hasActiveFocus())
        QGuiApplication::inputMethod()->update(Qt::ImEnabled);
    emit acceptsTextChanged();
}

void VideoView::inputMethodEvent(QInputMethodEvent *event)
{
    // Text being composed stays in the input method's own window; only
    // finished text goes to the phone.
    if (m_acceptsText && !event->commitString().isEmpty())
        emit textCommitted(event->commitString());
    event->accept();
}

QVariant VideoView::inputMethodQuery(Qt::InputMethodQuery query) const
{
    switch (query) {
    case Qt::ImEnabled:
        return m_acceptsText;
    case Qt::ImCursorRectangle: {
        // There is no caret to point at: the emoji panel and candidate
        // lists open at the middle of the phone's picture.
        const QPointF middle = pictureRect().center();
        return QRectF(middle.x(), middle.y(), 1, 1);
    }
    case Qt::ImSurroundingText:
        return QString();
    case Qt::ImCursorPosition:
    case Qt::ImAnchorPosition:
        return 0;
    default:
        return QQuickItem::inputMethodQuery(query);
    }
}

void VideoView::itemChange(ItemChange change, const ItemChangeData &value)
{
    QQuickItem::itemChange(change, value);
    if (change == ItemSceneChange && value.window) {
        connect(value.window, &QWindow::visibleChanged, this, [this](bool) { applyWindowChrome(); });
        applyWindowChrome();
    }
}

void VideoView::applyWindowChrome()
{
    QQuickWindow *win = window();
    if (!win)
        return;
#ifdef _WIN32
    if (!m_windowAppId.isEmpty()) {
        if (HWND hwnd = reinterpret_cast<HWND>(win->winId())) {
            IPropertyStore *store = nullptr;
            if (SUCCEEDED(SHGetPropertyStoreForWindow(hwnd, IID_PPV_ARGS(&store))) && store) {
                PROPVARIANT pv{};
                pv.vt = VT_LPWSTR;
                pv.pwszVal = const_cast<LPWSTR>(reinterpret_cast<LPCWSTR>(m_windowAppId.utf16()));
                store->SetValue(PKEY_AppUserModel_ID, pv);
                store->Commit();
                store->Release();
            }
        }
    }
#endif
    if (!m_windowIcon.isEmpty()) {
        const QUrl url(m_windowIcon);
        const QString path = url.isLocalFile() ? url.toLocalFile() : m_windowIcon;
        const QImage img(path);
        if (!img.isNull()) {
            QIcon icon;
            for (int sz : {16, 20, 24, 32, 40, 48, 64, 256}) {
                icon.addPixmap(QPixmap::fromImage(img.scaled(sz, sz, Qt::KeepAspectRatio, Qt::SmoothTransformation)));
            }
            win->setIcon(icon);
        }
    }
}

QRectF VideoView::pictureRect() const
{
    if (m_frameSize.isEmpty())
        return {};
    const QSizeF fitted = QSizeF(m_frameSize).scaled(size(), Qt::KeepAspectRatio);
    return QRectF((width() - fitted.width()) / 2, (height() - fitted.height()) / 2, fitted.width(), fitted.height());
}

void VideoView::frameArrived()
{
    refreshFrame(true);
}

void VideoView::refreshFrame(bool consumeQueuedWake)
{
    QSize size;
    {
        std::lock_guard guard(lock);
        if (consumeQueuedWake)
            queuedFrameUpdates.erase(this);
        size = latest.value(m_stream).image.size();
    }
    if (size != m_frameSize) {
        m_frameSize = size;
        emit frameSizeChanged();
    }
    ++m_frames;
    emit framesChanged();
    update();
}

QSGNode *VideoView::updatePaintNode(QSGNode *old, UpdatePaintNodeData *)
{
    Latest picture;
    {
        std::lock_guard guard(lock);
        picture = latest.value(m_stream);
    }
    if (picture.image.isNull() || !window()) {
        delete old;
        m_shown = 0;
        return nullptr;
    }
    auto *node = static_cast<VideoNode *>(old);
    if (!node)
        node = new VideoNode;
    if (picture.serial != m_shown || !node->texture()) {
        QSGTexture *texture = window()->createTextureFromImage(picture.image);
        texture->setFiltering(QSGTexture::Linear);
        node->replaceTexture(texture);
        m_shown = picture.serial;
    }
    node->setFiltering(QSGTexture::Linear);
    node->setRect(pictureRect());
    return node;
}

void video_frame(rust::Str stream, uint32_t width, uint32_t height, rust::Slice<const uint8_t> bgrx)
{
    if (bgrx.size() < size_t(width) * height * 4)
        return;
    // One copy, into an image Qt owns, performed outside the shared mutex.
    QImage image(reinterpret_cast<const uchar *>(bgrx.data()), int(width), int(height), int(width) * 4,
                 QImage::Format_RGB32);
    QImage copy = image.copy();
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.insert(key, Latest{std::move(copy), nextSerial++});
    wakeViews(key);
}

void video_clear(rust::Str stream)
{
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.remove(key);
    wakeViews(key);
}

void clear_idle_video_frames()
{
    std::lock_guard guard(lock);
    if (views.empty()) {
        latest.clear();
        latest.squeeze();
    }
}


// Actual native checks used only by the existing test-support window gate.
// They run on its GUI thread and use synthetic in-memory pixels, no Node or device.
rust::String video_wakeup_burst_check()
{
    auto *app = qobject_cast<QGuiApplication *>(QCoreApplication::instance());
    if (!app || QThread::currentThread() != app->thread())
        return rust::String("video wake burst check needs the GUI thread");
    const rust::Str stream("window-gate/video-wakeup-burst");
    const std::array<uint8_t, 24> pixels{};
    const rust::Slice<const uint8_t> data(pixels.data(), pixels.size());
    VideoView view;
    view.setStream(QStringLiteral("window-gate/video-wakeup-burst"));
    const quint64 before = view.frames();
    for (int i = 0; i < 256; ++i)
        video_frame(stream, 1, 1, data);
    video_frame(stream, 3, 2, data);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (view.frames() != before + 1 || view.frameSize() != QSize(3, 2))
        return rust::String("video wake burst was not coalesced to the latest frame");
    video_frame(stream, 2, 1, data);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (view.frames() != before + 2 || view.frameSize() != QSize(2, 1))
        return rust::String("video wake did not rearm after delivery");
    video_clear(stream);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    return {};
}

rust::String video_wakeup_lifecycle_check()
{
    auto *app = qobject_cast<QGuiApplication *>(QCoreApplication::instance());
    if (!app || QThread::currentThread() != app->thread())
        return rust::String("video wake lifecycle check needs the GUI thread");
    const rust::Str oldStream("window-gate/video-wakeup-old");
    const rust::Str newStream("window-gate/video-wakeup-new");
    const std::array<uint8_t, 24> pixels{};
    const rust::Slice<const uint8_t> data(pixels.data(), pixels.size());
    VideoView view;
    view.setStream(QStringLiteral("window-gate/video-wakeup-old"));
    const quint64 beforeClear = view.frames();
    video_frame(oldStream, 1, 1, data);
    video_clear(oldStream);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (view.frames() != beforeClear + 1 || !view.frameSize().isEmpty())
        return rust::String("video clear did not blank a queued frame");
    const quint64 afterClear = view.frames();
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (view.frames() != afterClear)
        return rust::String("video clear left a stale queued wake");
    video_frame(oldStream, 1, 1, data);
    view.setStream(QStringLiteral("window-gate/video-wakeup-new"));
    const quint64 beforeRebindDelivery = view.frames();
    video_frame(newStream, 2, 1, data);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (view.frames() != beforeRebindDelivery + 1 || view.frameSize() != QSize(2, 1))
        return rust::String("stream rebind consumed or duplicated a queued video wake");
    video_clear(oldStream);
    video_clear(newStream);
    QCoreApplication::sendPostedEvents(&view, QEvent::MetaCall);
    if (!view.frameSize().isEmpty())
        return rust::String("rebound video did not clear");
    return {};
}
