import Cocoa
import UniformTypeIdentifiers

// MARK: - Pasteboard writer (supports both modern file-url and legacy filenames type)

class FilePasteboardItem: NSObject, NSPasteboardWriting {
    let url: URL
    init(url: URL) { self.url = url }

    func writableTypes(for pasteboard: NSPasteboard) -> [NSPasteboard.PasteboardType] {
        return [.fileURL]
    }

    func pasteboardPropertyList(forType type: NSPasteboard.PasteboardType) -> Any? {
        if type == .fileURL { return url.absoluteString }
        return nil
    }
}

// MARK: - Draggable file view

class FileItemView: NSView {
    let url: URL
    private let label: NSTextField

    init(url: URL) {
        self.url = url
        self.label = NSTextField(labelWithString: url.lastPathComponent)
        super.init(frame: NSRect(x: 0, y: 0, width: 180, height: 36))

        label.frame = NSRect(x: 8, y: 8, width: 140, height: 20)
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = .white
        label.lineBreakMode = .byTruncatingMiddle
        addSubview(label)

        let closeBtn = NSButton(frame: NSRect(x: 156, y: 8, width: 20, height: 20))
        closeBtn.bezelStyle = .inline
        closeBtn.isBordered = false
        closeBtn.title = "✕"
        closeBtn.font = NSFont.systemFont(ofSize: 11)
        closeBtn.contentTintColor = NSColor(white: 0.6, alpha: 1)
        closeBtn.target = self
        closeBtn.action = #selector(remove)
        addSubview(closeBtn)

        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.backgroundColor = NSColor(white: 0.25, alpha: 1).cgColor
    }
    required init?(coder: NSCoder) { fatalError() }

    @objc func remove() {
        AppDelegate.shared.removeItem(url: url)
    }

    override func mouseDown(with event: NSEvent) {
        // Don't start drag if clicking the close button
        let point = convert(event.locationInWindow, from: nil)
        if point.x > 150 { return }
        let item = NSDraggingItem(pasteboardWriter: FilePasteboardItem(url: url))
        item.setDraggingFrame(bounds, contents: icon())
        beginDraggingSession(with: [item], event: event, source: self)
    }

    private func icon() -> NSImage {
        NSWorkspace.shared.icon(forFile: url.path)
    }
}

extension FileItemView: NSDraggingSource {
    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        return [.copy, .move, .link]
    }
    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        if !operation.isEmpty {
            DispatchQueue.main.async {
                AppDelegate.shared.removeItem(url: self.url)
            }
        }
    }
}

// MARK: - Shelf window

class ShelfWindow: NSPanel {
    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 0),
            styleMask: [.nonactivatingPanel, .fullSizeContentView, .borderless],
            backing: .buffered,
            defer: false
        )
        isOpaque = false
        backgroundColor = NSColor(white: 0.12, alpha: 0.95)
        level = .floating
        isMovableByWindowBackground = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        hasShadow = true
        contentView?.wantsLayer = true
        contentView?.layer?.cornerRadius = 10
        contentView?.layer?.masksToBounds = true
        // Position top-right
        if let screen = NSScreen.main {
            let x = screen.visibleFrame.maxX - 220
            let y = screen.visibleFrame.maxY - 60
            setFrameOrigin(NSPoint(x: x, y: y))
        }
    }
}

// MARK: - App delegate

class AppDelegate: NSObject, NSApplicationDelegate {
    static var shared: AppDelegate!
    var window: ShelfWindow!
    var items: [FileItemView] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        AppDelegate.shared = self

        window = ShelfWindow()


        // Listen for files added via CLI: `shelf-add /path/to/file`
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(addFromNotification(_:)),
            name: NSNotification.Name("com.dir-viewer.shelf.add"),
            object: nil
        )
    }

    @objc func addFromNotification(_ note: Notification) {
        guard let path = note.object as? String else { return }
        let url = URL(fileURLWithPath: path)
        DispatchQueue.main.async { self.addItem(url: url) }
    }

    func addItem(url: URL) {
        guard !items.contains(where: { $0.url == url }) else { return }
        let view = FileItemView(url: url)
        items.append(view)
        relayout()
        if !window.isVisible { window.orderFront(nil) }
    }

    func removeItem(url: URL) {
        items.removeAll { $0.url == url }
        relayout()
        if items.isEmpty { window.orderOut(nil) }
    }

    @objc func clearAll() {
        items.removeAll()
        relayout()
        window.orderOut(nil)
    }

    @objc func quit() { NSApp.terminate(nil) }

    private func relayout() {
        let padding: CGFloat = 8
        let itemH: CGFloat = 36
        let totalH = items.isEmpty ? 0 : CGFloat(items.count) * (itemH + padding) + padding
        let contentView = window.contentView!
        contentView.subviews.forEach { $0.removeFromSuperview() }

        var y = padding
        for view in items.reversed() {
            view.frame = NSRect(x: padding, y: y, width: 184, height: itemH)
            contentView.addSubview(view)
            y += itemH + padding
        }

        var frame = window.frame
        let oldH = frame.height
        frame.size.height = totalH
        frame.origin.y += oldH - totalH
        window.setFrame(frame, display: true, animate: false)
    }
}

// MARK: - Entry point

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
