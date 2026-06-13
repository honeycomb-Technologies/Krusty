import PhotosUI
import UIKit
import UniformTypeIdentifiers

public struct PickedAttachment: Sendable, Equatable {
    public var name: String
    public var mimeType: String?
    public var data: Data?
    public var url: URL?

    public init(name: String, mimeType: String? = nil, data: Data? = nil, url: URL? = nil) {
        self.name = name
        self.mimeType = mimeType
        self.data = data
        self.url = url
    }
}

@MainActor
public final class AttachmentPicker: NSObject, PHPickerViewControllerDelegate, UIDocumentPickerDelegate {
    public var onPick: ((PickedAttachment) -> Void)?
    public var onCancel: (() -> Void)?

    public func photoPicker() -> PHPickerViewController {
        var configuration = PHPickerConfiguration(photoLibrary: .shared())
        configuration.filter = .images
        configuration.selectionLimit = 1
        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = self
        return picker
    }

    public func documentPicker() -> UIDocumentPickerViewController {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: true)
        picker.delegate = self
        picker.allowsMultipleSelection = false
        return picker
    }

    public func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let provider = results.first?.itemProvider else {
            onCancel?()
            return
        }
        let type = UTType.image.identifier
        provider.loadDataRepresentation(forTypeIdentifier: type) { [weak self] data, _ in
            guard let data else { return }
            Task { @MainActor in
                self?.onPick?(PickedAttachment(name: "image", mimeType: "image/*", data: data))
            }
        }
    }

    public func documentPicker(
        _ controller: UIDocumentPickerViewController,
        didPickDocumentsAt urls: [URL]
    ) {
        guard let url = urls.first else {
            onCancel?()
            return
        }
        onPick?(PickedAttachment(name: url.lastPathComponent, url: url))
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        onCancel?()
    }
}
