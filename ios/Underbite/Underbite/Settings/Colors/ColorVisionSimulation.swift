#if DEBUG
    import CoreImage
    import CoreImage.CIFilterBuiltins
    import UIKit
    import UnderbiteKit

    /// `-simulate protan|deutan|tritan|achromat` (screenshots, debug builds only): a window over
    /// the app showing the screen as a reader with that colour vision sees it, with the
    /// matrices the theme checks use (`ColorMath.simulationMatrix`, Machado et al. 2009 at full
    /// severity), applied by Core Image in its linear working space. It redraws twice a second
    /// from a snapshot of the app's window (the map included) and lets every touch through. A
    /// strip at the bottom, under the picture (drawn a little smaller to make room), says which simulation it is.
    @MainActor
    enum ColorVisionSimulation {
        private static var window: UIWindow?
        private static var imageView: UIImageView?
        private static let context = CIContext()
        /// The height of the label's strip under the picture.
        private static let stripHeight: CGFloat = 22

        /// Starts the simulation when the launch asks for one.
        static func startIfAsked() {
            guard window == nil,
                let deficiency = LaunchArguments.value(after: "-simulate").flatMap(Deficiency.init(rawValue:)),
                let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first
            else { return }
            let overlay = UIWindow(windowScene: scene)
            overlay.windowLevel = .alert + 1
            overlay.isUserInteractionEnabled = false
            overlay.backgroundColor = .black
            let controller = UIViewController()
            controller.view.backgroundColor = .black
            // The label has a strip of its own under the picture, which is drawn a little
            // smaller to make room, so the label covers nothing of the app.
            let strip = UIView()
            strip.backgroundColor = .black
            strip.translatesAutoresizingMaskIntoConstraints = false
            let image = UIImageView()
            image.contentMode = .scaleAspectFit
            image.clipsToBounds = true
            image.translatesAutoresizingMaskIntoConstraints = false
            controller.view.addSubview(image)
            controller.view.addSubview(strip)
            let badge = UILabel()
            badge.text = "Simulated: \(deficiency.rawValue)"
            badge.font = .preferredFont(forTextStyle: .caption2)
            badge.textColor = .white
            badge.translatesAutoresizingMaskIntoConstraints = false
            strip.addSubview(badge)
            let view: UIView = controller.view
            NSLayoutConstraint.activate([
                strip.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                strip.trailingAnchor.constraint(equalTo: view.trailingAnchor),
                strip.bottomAnchor.constraint(equalTo: view.bottomAnchor),
                strip.heightAnchor.constraint(equalToConstant: Self.stripHeight),
                image.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                image.trailingAnchor.constraint(equalTo: view.trailingAnchor),
                image.topAnchor.constraint(equalTo: view.topAnchor),
                image.bottomAnchor.constraint(equalTo: strip.topAnchor),
                badge.centerXAnchor.constraint(equalTo: strip.centerXAnchor),
                badge.centerYAnchor.constraint(equalTo: strip.centerYAnchor),
            ])
            overlay.rootViewController = controller
            overlay.isHidden = false
            window = overlay
            imageView = image
            Task {
                while !Task.isCancelled {
                    redraw(deficiency, scene: scene)
                    try? await Task.sleep(for: .milliseconds(500))
                }
            }
        }

        /// Snapshots the app's window and shows it through the simulation.
        private static func redraw(_ deficiency: Deficiency, scene: UIWindowScene) {
            guard let imageView,
                let app = scene.keyWindow ?? scene.windows.first(where: { $0 !== window && !$0.isHidden })
            else { return }
            let shot = UIGraphicsImageRenderer(bounds: app.bounds).image { _ in
                _ = app.drawHierarchy(in: app.bounds, afterScreenUpdates: false)
            }
            guard let input = CIImage(image: shot) else { return }
            let m = ColorMath.simulationMatrix(deficiency)
            let filter = CIFilter.colorMatrix()
            filter.inputImage = input
            filter.rVector = CIVector(x: m[0][0], y: m[0][1], z: m[0][2], w: 0)
            filter.gVector = CIVector(x: m[1][0], y: m[1][1], z: m[1][2], w: 0)
            filter.bVector = CIVector(x: m[2][0], y: m[2][1], z: m[2][2], w: 0)
            filter.aVector = CIVector(x: 0, y: 0, z: 0, w: 1)
            filter.biasVector = CIVector(x: 0, y: 0, z: 0, w: 0)
            guard let output = filter.outputImage?.applyingFilter("CIColorClamp"),
                let image = context.createCGImage(output, from: input.extent)
            else { return }
            imageView.image = UIImage(cgImage: image, scale: shot.scale, orientation: .up)
        }
    }
#endif
