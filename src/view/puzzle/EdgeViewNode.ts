import { TRedLineStyle } from '../Theme.ts';
import { ChunkedPaths, Chunking } from './ChunkedPaths.ts';
import { TPuzzleStyle } from './TPuzzleStyle.ts';

import { Multilink, TReadOnlyProperty } from 'scenerystack/axon';
import { Vector2 } from 'scenerystack/dot';
import { Node } from 'scenerystack/scenery';

import { TBoard } from '../../model/board/core/TBoard.ts';
import { TState } from '../../model/data/core/TState.ts';
import EdgeState from '../../model/data/edge-state/EdgeState.ts';
import { TEdgeStateData } from '../../model/data/edge-state/TEdgeStateData.ts';

export class EdgeViewNode extends Node {
  public constructor(
    board: TBoard,
    stateProperty: TReadOnlyProperty<TState<TEdgeStateData>>,
    isSolvedProperty: TReadOnlyProperty<boolean>,
    style: TPuzzleStyle,
  ) {
    // Drawn in chunks of nearby edges, so a move only rebuilds the chunks it changed
    const chunking = new Chunking(
      board.edges.map((edge) => edge.start.viewCoordinates.blend(edge.end.viewCoordinates, 0.5)),
    );

    const whitePath = new ChunkedPaths(chunking, {
      lineWidth: 0.02,
      stroke: style.theme.whiteLineColorProperty,
    });

    const redXPath = new ChunkedPaths(chunking, {
      stroke: style.theme.xColorProperty,
      lineWidth: 0.025,
    });

    const redLinePath = new ChunkedPaths(chunking, {
      lineWidth: 0.02,
      fill: style.theme.redLineColorProperty,
    });
    const paths = [whitePath, redXPath, redLinePath];

    super({
      children: [whitePath, redXPath, redLinePath],
      pickable: false,
    });

    // NOTE: this is actually a valid start state
    const edgeStates: EdgeState[] = board.edges.map((edge) => EdgeState.BLACK); // Used because we currently don't display BLACK!
    let alignedXs = false;

    let showedWhiteLines = false;
    let showedRedXs = false;
    let showedRedLines = false;
    let showedRedLineStyle: TRedLineStyle | null = null;

    const multilink = Multilink.multilink(
      [
        stateProperty,
        isSolvedProperty,
        style.whiteLineVisibleProperty,
        style.redXsVisibleProperty,
        style.redXsAlignedProperty,
        style.redLineVisibleProperty,
        style.redLineStyleProperty,
      ],
      (state, isSolved, whiteLineVisible, redXsVisible, redXsAligned, redLineVisible, redLineStyle) => {
        this.visible = !isSolved;
        whitePath.visible = whiteLineVisible;
        redXPath.visible = redXsVisible;
        redLinePath.visible = redLineVisible;

        if (this.visible) {
          let changed = false;
          for (let i = 0; i < board.edges.length; i++) {
            const edgeState = state.getEdgeState(board.edges[i]);

            if (edgeState !== edgeStates[i]) {
              changed = true;
              edgeStates[i] = edgeState;
              paths.forEach((path) => path.markItemDirty(i));
            }
          }

          // Style changes redraw everything
          let styleChanged = false;
          if (alignedXs !== redXsAligned) {
            styleChanged = true;
            alignedXs = redXsAligned;
          }
          if (showedWhiteLines !== whiteLineVisible) {
            styleChanged = true;
            showedWhiteLines = whiteLineVisible;
          }
          if (showedRedXs !== redXsVisible) {
            styleChanged = true;
            showedRedXs = redXsVisible;
          }
          if (showedRedLines !== redLineVisible) {
            styleChanged = true;
            showedRedLines = redLineVisible;
          }
          if (showedRedLineStyle !== redLineStyle) {
            styleChanged = true;
            showedRedLineStyle = redLineStyle;
          }

          if (styleChanged) {
            changed = true;
            paths.forEach((path) => path.markAllDirty());
          }

          if (changed) {
            whitePath.update((whiteShape, i) => {
              if (whiteLineVisible && edgeStates[i] === EdgeState.WHITE) {
                const edge = board.edges[i];
                whiteShape.moveTo(edge.start.viewCoordinates.x, edge.start.viewCoordinates.y);
                whiteShape.lineTo(edge.end.viewCoordinates.x, edge.end.viewCoordinates.y);
              }
            });

            redXPath.update((redXShape, i) => {
              if (edgeStates[i] === EdgeState.RED) {
                if (redXsVisible) {
                  const edge = board.edges[i];
                  const halfSize = 0.07;

                  let center = edge.start.viewCoordinates.blend(edge.end.viewCoordinates, 0.5);

                  if (redXsAligned) {
                    const angle = edge.end.viewCoordinates.minus(edge.start.viewCoordinates).getAngle();

                    const a = new Vector2(-halfSize, -halfSize).rotate(angle).add(center);
                    const b = new Vector2(halfSize, halfSize).rotate(angle).add(center);
                    const c = new Vector2(-halfSize, halfSize).rotate(angle).add(center);
                    const d = new Vector2(halfSize, -halfSize).rotate(angle).add(center);

                    redXShape.moveTo(a.x, a.y);
                    redXShape.lineTo(b.x, b.y);
                    redXShape.moveTo(c.x, c.y);
                    redXShape.lineTo(d.x, d.y);
                  } else {
                    redXShape.moveTo(center.x - halfSize, center.y - halfSize);
                    redXShape.lineTo(center.x + halfSize, center.y + halfSize);
                    redXShape.moveTo(center.x - halfSize, center.y + halfSize);
                    redXShape.lineTo(center.x + halfSize, center.y - halfSize);
                  }
                }
              }
            });

            redLinePath.update((redLineShape, i) => {
              if (edgeStates[i] === EdgeState.RED) {
                if (redLineVisible) {
                  // TODO: ditch style property?

                  const edge = board.edges[i];
                  const redLineProportion = 0.4;
                  const radius = 0.017;
                  const center = edge.start.viewCoordinates.blend(edge.end.viewCoordinates, 0.5);
                  const pointA = center.blend(edge.start.viewCoordinates, redLineProportion);
                  const pointB = center.blend(edge.end.viewCoordinates, redLineProportion);

                  for (let j = 0; j < 5; j++) {
                    const point = pointA.blend(pointB, j / 4);

                    redLineShape.moveTo(point.x + radius, point.y);
                    redLineShape.arc(point.x, point.y, radius, 0, 2 * Math.PI, false);
                  }
                }
              }
            });
          }
        }
      },
    );
    this.disposeEmitter.addListener(() => multilink.dispose());
  }
}
