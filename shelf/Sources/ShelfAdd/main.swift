import Foundation

guard CommandLine.arguments.count > 1 else {
    fputs("Usage: shelf-add <path>\n", stderr)
    exit(1)
}

let path = CommandLine.arguments[1]

DistributedNotificationCenter.default().postNotificationName(
    NSNotification.Name("com.dir-viewer.shelf.add"),
    object: path,
    userInfo: nil,
    deliverImmediately: true
)

// Give the notification time to deliver
Thread.sleep(forTimeInterval: 0.1)
