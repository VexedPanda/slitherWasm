import PuzzleModelNode from './PuzzleModelNode.ts';
import TopologicalPuzzleNode from './TopologicalPuzzleNode.ts';
import { TPuzzleStyle } from './puzzle/TPuzzleStyle.ts';
import { hookPuzzleBackgroundListeners } from './puzzle/hookPuzzleBackgroundListeners.ts';
import { currentPuzzleStyle } from './puzzle/puzzleStyles.ts';

import { Multilink, TReadOnlyProperty } from 'scenerystack/axon';
import { Bounds2, Matrix3, Vector2 } from 'scenerystack/dot';
import { Shape } from 'scenerystack/kite';
import { optionize } from 'scenerystack/phet-core';
import {
  AnimatedPanZoomListener,
  LinearGradient,
  Node,
  NodeOptions,
  RadialGradient,
  Rectangle,
  Sizable,
  SizableOptions,
} from 'scenerystack/scenery';

import { showLayoutTestProperty } from '../model/board/layout/layout.ts';
import { isFaceColorOutsideAvailableEditModeProperty } from '../model/puzzle/EditMode.ts';
import PuzzleModel from '../model/puzzle/PuzzleModel.ts';

type SelfOptions = {
  topological?: boolean;
};

type ParentOptions = SizableOptions & NodeOptions;

export type PuzzleContainerNodeOptions = SelfOptions & ParentOptions;

const PUZZLE_PADDING = 20;

// By default, zoom in so the puzzle is this many times the usable height of the play area
const DEFAULT_ZOOM_HEIGHT_MULTIPLIER = 2;

class PuzzlePanZoomListener extends AnimatedPanZoomListener {
  // Immediately (without animating) scales the target about a point in its parent's coordinate frame
  public jumpToScale(scale: number, center: Vector2): void {
    this.matrixProperty.value = Matrix3.translation(center.x, center.y)
      .timesMatrix(Matrix3.scaling(this.limitScale(scale)))
      .timesMatrix(Matrix3.translation(-center.x, -center.y));
    this.correctReposition();

    // Private in the type declarations, but needed so the animation doesn't pull us back to the previous scale
    (this as unknown as { stopInProgressAnimation(): void }).stopInProgressAnimation();
  }
}

export default class PuzzleContainerNode extends Sizable(Node) {
  private backgroundRect: Rectangle;
  private facePressRect: Rectangle;
  private rectangularGradientRect: Rectangle;
  private circularGradientRect: Rectangle;
  private puzzleWrapper: Node;
  private puzzleNode: PuzzleModelNode | null = null;
  private zoomListener: PuzzlePanZoomListener;

  // The transform from the last time we applied the default zoom. While the user hasn't changed it, we'll reapply the
  // default zoom on layout changes. null if we need to apply it (e.g. a new puzzle).
  private defaultZoomMatrix: Matrix3 | null = null;

  public constructor(
    public readonly puzzleModelProperty: TReadOnlyProperty<PuzzleModel | null>,
    public readonly style: TPuzzleStyle = currentPuzzleStyle,
    providedOptions?: PuzzleContainerNodeOptions,
  ) {
    const options = optionize<PuzzleContainerNodeOptions, SelfOptions, ParentOptions>()(
      {
        topological: false,
      },
      providedOptions,
    );

    // TODO: isolate out these options
    super(options);

    this.backgroundRect = new Rectangle({
      fill: style.theme.playAreaBackgroundColorProperty,
      pickable: true,
    });

    // Invisible rectangle for face press events
    this.facePressRect = new Rectangle({
      fill: null,
      pickableProperty: isFaceColorOutsideAvailableEditModeProperty,
    });

    // TODO: adjust pickable based on edit mode

    hookPuzzleBackgroundListeners(
      this.facePressRect,
      (button) => {
        puzzleModelProperty.value?.onUserFacePress(null, button);
      },
      (event) => {
        this.puzzleNode?.puzzleNode.onFaceBackgroundDragStart(event);
      },
    );

    this.rectangularGradientRect = new Rectangle({
      pickable: false,
    });
    this.circularGradientRect = new Rectangle({
      pickable: false,
    });

    this.puzzleWrapper = new Node({
      children: [this.backgroundRect, this.facePressRect, this.rectangularGradientRect, this.circularGradientRect],
    });
    this.zoomListener = new PuzzlePanZoomListener(this.puzzleWrapper, {
      maxScale: 10,
    });
    // display.addInputListener( this.zoomListener );
    this.addInputListener(this.zoomListener);

    this.addChild(this.puzzleWrapper);

    const layoutListener = this.updateLayout.bind(this);
    this.localPreferredWidthProperty.lazyLink(layoutListener);
    this.localPreferredHeightProperty.lazyLink(layoutListener);

    if (options.topological) {
      // TODO: layout still not working great?
      this.puzzleWrapper.localBoundsProperty.lazyLink(layoutListener);
    }

    // TODO: generalize this just as a container node, without this specialization for creating the type?
    Multilink.multilink(
      [
        puzzleModelProperty,
        showLayoutTestProperty, // TODO: rename?
      ],
      (puzzleModel, showLayout) => {
        if (this.puzzleNode) {
          this.puzzleNode.dispose();
          this.puzzleNode = null;
        }

        if (puzzleModel) {
          if (options.topological) {
            if (showLayout) {
              // TODO: omg this hackery! We are... relying on STRUCTURAL typing of these two types to be equivalent?
              this.puzzleNode = new TopologicalPuzzleNode(puzzleModel);
            }
          } else {
            this.puzzleNode = new PuzzleModelNode(puzzleModel, {
              focusNodeCallback: (node: Node) => {
                this.zoomListener.panToNode(node, true);
              },
            });
          }

          if (this.puzzleNode) {
            // Update before children, so we don't mess with layout
            this.updatePuzzleNodeLayout(this.puzzleNode);
            this.puzzleWrapper.children = [
              this.backgroundRect,
              this.facePressRect,
              this.rectangularGradientRect,
              this.circularGradientRect,
              this.puzzleNode,
            ];
          }
        }

        this.zoomListener.resetTransform();
        this.defaultZoomMatrix = null;

        this.updateLayout();
      },
    );
  }

  public step(dt: number): void {
    this.zoomListener.step(2 * dt);
  }

  private updateLayout(): void {
    const width = this.localPreferredWidth;
    const height = this.localPreferredHeight;

    if (width !== null) {
      this.backgroundRect.rectWidth = width;
      this.facePressRect.rectWidth = width;
      this.rectangularGradientRect.rectWidth = width;
      this.circularGradientRect.rectWidth = width;
    }
    if (height !== null) {
      this.backgroundRect.rectHeight = height;
      this.facePressRect.rectHeight = height;
      this.rectangularGradientRect.rectHeight = height;
      this.circularGradientRect.rectHeight = height;
    }

    if (this.puzzleNode) {
      this.updatePuzzleNodeLayout(this.puzzleNode);
    }

    if (width !== null && height !== null && width > 0 && height > 0) {
      const bounds = new Bounds2(0, 0, width, height);

      const center = bounds.center;
      const distanceToCorner = center.getMagnitude();

      this.rectangularGradientRect.fill = new LinearGradient(0, 0, 0, height)
        .addColorStop(0, this.style.theme.playAreaLinearTopColorProperty)
        .addColorStop(0.5, this.style.theme.playAreaLinearMiddleColorProperty)
        .addColorStop(1, this.style.theme.playAreaLinearBottomColorProperty);

      this.circularGradientRect.fill = new RadialGradient(center.x, center.y, distanceToCorner, center.x, center.y, 0)
        .addColorStop(0, this.style.theme.playAreaRadialOutsideColorProperty)
        .addColorStop(1, this.style.theme.playAreaRadialInsideColorProperty);

      this.clipArea = Shape.bounds(bounds);

      // zoomListener.setTargetScale( scale );
      // TODO: we're getting weird oscillation with this, but figure it out
      // this.zoomListener.setTargetBounds( this.puzzleNode ? this.puzzleNode.bounds : bounds );
      this.zoomListener.setTargetBounds(bounds);
      this.zoomListener.setPanBounds(bounds);

      this.applyDefaultZoom(width, height);
    } else {
      const workaroundBounds = new Bounds2(0, 0, 5, 5);
      this.zoomListener.setTargetBounds(workaroundBounds);
      this.zoomListener.setPanBounds(workaroundBounds);
    }
  }

  private applyDefaultZoom(width: number, height: number): void {
    if (this.defaultZoomMatrix && !this.defaultZoomMatrix.equalsEpsilon(this.puzzleWrapper.matrix, 1e-8)) {
      // The user has zoomed or panned, don't override that
      return;
    }

    if (!this.puzzleNode || !this.puzzleNode.bounds.isValid() || this.puzzleNode.bounds.height <= 0) {
      return;
    }

    // Bounds of the puzzle when not zoomed
    const puzzleHeight = this.puzzleNode.bounds.height;
    const usableHeight = height - PUZZLE_PADDING * 2;
    if (usableHeight <= 0) {
      return;
    }

    const scale = Math.max(1, (DEFAULT_ZOOM_HEIGHT_MULTIPLIER * usableHeight) / puzzleHeight);
    this.zoomListener.jumpToScale(scale, new Vector2(width / 2, height / 2));
    this.defaultZoomMatrix = this.puzzleWrapper.matrix.copy();
  }

  private updatePuzzleNodeLayout(puzzleNode: PuzzleModelNode): void {
    const width = this.localPreferredWidth;
    const height = this.localPreferredHeight;

    if (width !== null && height !== null && width > 0 && height > 0) {
      const availableWidth = width - PUZZLE_PADDING * 2;
      const availableHeight = height - PUZZLE_PADDING * 2;

      if (puzzleNode.localBounds.isValid()) {
        const puzzleWidth = puzzleNode.localBounds.width;
        const puzzleHeight = puzzleNode.localBounds.height;

        const scale = Math.min(availableWidth / puzzleWidth, availableHeight / puzzleHeight);

        puzzleNode.setScaleMagnitude(scale);
        puzzleNode.center = new Vector2(width / 2, height / 2);
      }
    }
  }
}
