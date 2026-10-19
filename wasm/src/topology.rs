//! Half-edge topology of a board, in the same orders as the TS board (several HARD rules depend on cyclic and index
//! orders, see createBoardDescriptor.ts):
//!
//! - face.halfEdges[i] goes from face.vertices[i] to face.vertices[i + 1], and face.edges[i] is its edge
//! - vertex.incomingHalfEdges is cyclic, and vertex.edges[k] is incomingHalfEdges[k]'s edge
//! - a sector is a half-edge h: the corner at h's end, between h's edge and h.next's edge, in h's face (or the outside)

pub const NONE: u32 = u32::MAX;

pub struct Topology {
    pub num_vertices: usize,
    pub num_edges: usize,
    pub num_faces: usize,
    pub num_half_edges: usize,
    pub he_start: Vec<u32>,
    pub he_end: Vec<u32>,
    pub he_edge: Vec<u32>,
    pub he_face: Vec<u32>, // NONE for the outside
    pub he_next: Vec<u32>,
    pub he_reversed: Vec<u32>,
    pub he_previous: Vec<u32>,
    // Per vertex (CSR): incoming half-edges (cyclic), and their edges (vertex.edges)
    pub vertex_offsets: Vec<u32>,
    pub vertex_incoming: Vec<u32>,
    pub vertex_edges: Vec<u32>,
    // Per face (CSR): half-edges, edges and vertices, in face order
    pub face_offsets: Vec<u32>,
    pub face_half_edges: Vec<u32>,
    pub face_edges: Vec<u32>,
    pub face_vertices: Vec<u32>,
    // One half-edge of each edge
    pub edge_half_edge: Vec<u32>,
    // Faces sharing an edge with each face (CSR, no duplicates)
    pub face_neighbor_offsets: Vec<u32>,
    pub face_neighbors: Vec<u32>,
    // Faces around each vertex (CSR, no duplicates)
    pub vertex_face_offsets: Vec<u32>,
    pub vertex_faces: Vec<u32>,
    // Edges, vertices and faces on the outside boundary
    pub boundary_edges: Vec<u32>,
    pub boundary_vertices: Vec<u32>,
    pub boundary_faces: Vec<u32>,
}

impl Topology {
    /// From the half-edge arrays (start, end, edge, face (NONE for outside), next, reversed per half-edge), each vertex's
    /// incoming half-edges (CSR), and each face's half-edges (CSR).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_vertices: usize,
        num_edges: usize,
        num_faces: usize,
        half_edges: &[u32],
        vertex_offsets: &[u32],
        vertex_incoming: &[u32],
        face_offsets: &[u32],
        face_half_edges: &[u32],
    ) -> Topology {
        let num_half_edges = half_edges.len() / 6;
        let field = |k: usize| (0..num_half_edges).map(|h| half_edges[6 * h + k]).collect::<Vec<u32>>();
        let he_start = field(0);
        let he_end = field(1);
        let he_edge = field(2);
        let he_face = field(3);
        let he_next = field(4);
        let he_reversed = field(5);
        let mut he_previous = vec![0; num_half_edges];
        for h in 0..num_half_edges {
            he_previous[he_next[h] as usize] = h as u32;
        }
        let vertex_edges = vertex_incoming.iter().map(|&h| he_edge[h as usize]).collect();
        let face_edges = face_half_edges.iter().map(|&h| he_edge[h as usize]).collect();
        let face_vertices = face_half_edges.iter().map(|&h| he_start[h as usize]).collect();
        let mut edge_half_edge = vec![NONE; num_edges];
        for h in 0..num_half_edges {
            if edge_half_edge[he_edge[h] as usize] == NONE {
                edge_half_edge[he_edge[h] as usize] = h as u32;
            }
        }

        let mut face_neighbor_offsets = vec![0u32];
        let mut face_neighbors = Vec::new();
        for f in 0..num_faces {
            let start = face_neighbors.len();
            for i in face_offsets[f]..face_offsets[f + 1] {
                let h = face_half_edges[i as usize] as usize;
                let other = he_face[he_reversed[h] as usize];
                if other != NONE && !face_neighbors[start..].contains(&other) {
                    face_neighbors.push(other);
                }
            }
            face_neighbor_offsets.push(face_neighbors.len() as u32);
        }

        let mut vertex_face_offsets = vec![0u32];
        let mut vertex_faces = Vec::new();
        for v in 0..num_vertices {
            let start = vertex_faces.len();
            for i in vertex_offsets[v]..vertex_offsets[v + 1] {
                let f = he_face[vertex_incoming[i as usize] as usize];
                if f != NONE && !vertex_faces[start..].contains(&f) {
                    vertex_faces.push(f);
                }
            }
            vertex_face_offsets.push(vertex_faces.len() as u32);
        }

        let mut boundary_edges = Vec::new();
        let mut boundary_vertices = Vec::new();
        let mut boundary_faces = Vec::new();
        for h in 0..num_half_edges {
            if he_face[h] == NONE {
                boundary_edges.push(he_edge[h]);
                for v in [he_start[h], he_end[h]] {
                    if !boundary_vertices.contains(&v) {
                        boundary_vertices.push(v);
                    }
                }
                let f = he_face[he_reversed[h] as usize];
                if f != NONE && !boundary_faces.contains(&f) {
                    boundary_faces.push(f);
                }
            }
        }

        Topology {
            num_vertices,
            num_edges,
            num_faces,
            num_half_edges,
            he_start,
            he_end,
            he_edge,
            he_face,
            he_next,
            he_reversed,
            he_previous,
            vertex_offsets: vertex_offsets.to_vec(),
            vertex_incoming: vertex_incoming.to_vec(),
            vertex_edges,
            face_offsets: face_offsets.to_vec(),
            face_half_edges: face_half_edges.to_vec(),
            face_edges,
            face_vertices,
            edge_half_edge,
            face_neighbor_offsets,
            face_neighbors,
            vertex_face_offsets,
            vertex_faces,
            boundary_edges,
            boundary_vertices,
            boundary_faces,
        }
    }

    #[inline]
    pub fn vertex_range(&self, v: usize) -> std::ops::Range<usize> {
        self.vertex_offsets[v] as usize..self.vertex_offsets[v + 1] as usize
    }

    #[inline]
    pub fn degree(&self, v: usize) -> usize {
        (self.vertex_offsets[v + 1] - self.vertex_offsets[v]) as usize
    }

    #[inline]
    pub fn edges_of_vertex(&self, v: usize) -> &[u32] {
        &self.vertex_edges[self.vertex_range(v)]
    }

    #[inline]
    pub fn incoming_of_vertex(&self, v: usize) -> &[u32] {
        &self.vertex_incoming[self.vertex_range(v)]
    }

    #[inline]
    pub fn face_range(&self, f: usize) -> std::ops::Range<usize> {
        self.face_offsets[f] as usize..self.face_offsets[f + 1] as usize
    }

    #[inline]
    pub fn edges_of_face(&self, f: usize) -> &[u32] {
        &self.face_edges[self.face_range(f)]
    }

    #[inline]
    pub fn half_edges_of_face(&self, f: usize) -> &[u32] {
        &self.face_half_edges[self.face_range(f)]
    }

    #[inline]
    pub fn vertices_of_face(&self, f: usize) -> &[u32] {
        &self.face_vertices[self.face_range(f)]
    }

    #[inline]
    pub fn neighbors_of_face(&self, f: usize) -> &[u32] {
        &self.face_neighbors[self.face_neighbor_offsets[f] as usize..self.face_neighbor_offsets[f + 1] as usize]
    }

    #[inline]
    pub fn faces_of_vertex(&self, v: usize) -> &[u32] {
        &self.vertex_faces[self.vertex_face_offsets[v] as usize..self.vertex_face_offsets[v + 1] as usize]
    }

    #[inline]
    pub fn other_vertex(&self, e: u32, v: u32) -> u32 {
        let h = self.edge_half_edge[e as usize] as usize;
        if self.he_start[h] == v {
            self.he_end[h]
        } else {
            self.he_start[h]
        }
    }

    /// The two faces of an edge (NONE for the outside)
    #[inline]
    pub fn faces_of_edge(&self, e: u32) -> [u32; 2] {
        let h = self.edge_half_edge[e as usize] as usize;
        [self.he_face[h], self.he_face[self.he_reversed[h] as usize]]
    }

    /// The face across edge e from face f (NONE for the outside)
    #[inline]
    pub fn other_face(&self, e: u32, f: u32) -> u32 {
        let [a, b] = self.faces_of_edge(e);
        if a == f { b } else { a }
    }
}
